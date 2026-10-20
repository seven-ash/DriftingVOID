//! Full-resolution CPU compositor (tiny-skia). Used for export, clipboard copy
//! and the eyedropper, so all three agree with each other.

use std::collections::HashMap;
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

/// The visible mask layer of a layer, if it has one.
pub fn mask_for(doc: &Document, id: crate::doc::LayerId) -> Option<&Layer> {
    doc.layers.iter().find(|m| m.mask_of == Some(id) && m.visible && matches!(m.kind, LayerKind::Raster(_)))
}

/// Where a mask's pixels show now: it was painted with its anchor at `last`
/// (centre), turned `pr` and sized `pe`; the anchor is now at `c`, turned by
/// `th` more and scaled by `s`. Identity when the mask is in step (sync after
/// edits); during playback and in video exports the mask follows through this.
#[derive(Clone, Copy, Debug)]
pub struct MaskMap {
    pub last: Pos2,
    pub c: Pos2,
    pub th: f32,
    pub s: f32,
}

impl MaskMap {
    /// Mask-space point -> where it shows.
    pub fn fwd(&self, p: Pos2) -> Pos2 {
        self.c + (crate::doc::rot_about(p, self.last, self.th) - self.last) * self.s
    }

    /// Shown point -> mask space.
    pub fn inv(&self, p: Pos2) -> Pos2 {
        crate::doc::rot_about(self.last + (p - self.c) / self.s.max(1e-6), self.last, -self.th)
    }
}

/// The live transform of an anchored mask (None: not anchored or in step).
pub fn mask_map(doc: &Document, mask: &Layer) -> Option<MaskMap> {
    let (aid, last) = mask.mask_anchor?;
    let [pr, pe] = mask.mask_pose?;
    let a = doc.get(aid)?;
    let k = crate::anim::key_of(a, 0.0);
    let c = egui::pos2(k.center[0], k.center[1]);
    let last = egui::pos2(last[0], last[1]);
    let s = if pe.abs() > 1e-6 && k.ext[0].abs() > 1e-6 { k.ext[0] / pe } else { 1.0 };
    let th = k.rot - pr;
    let m = MaskMap { last, c, th, s };
    let moved = (c - last).length() > 1e-3 || th.abs() > 1e-5 || (s - 1.0).abs() > 1e-5;
    moved.then_some(m)
}

/// How much of a point the mask hides (0..1): painted coverage (or its
/// opposite when inverted), times the mask layer's opacity (its strength).
pub fn mask_hide(mask: &Layer, map: Option<MaskMap>, p: Pos2) -> f32 {
    let LayerKind::Raster(r) = &mask.kind else { return 0.0 };
    let q = map.map_or(p, |m| m.inv(p));
    let a = r.alpha_at_world(q) as f32 / 255.0;
    let a = if mask.mask_invert { 1.0 - a } else { a };
    a * mask.opacity.clamp(0.0, 1.0)
}

/// Hides what a mask covers (both in canvas space).
pub fn apply_mask(pm: &mut Pixmap, doc: &Document, m: &Layer, region: Rect, scale: f32) {
    let map = mask_map(doc, m);
    let w = pm.width() as usize;
    let data = pm.data_mut();
    for (i, px) in data.chunks_exact_mut(4).enumerate() {
        if px[3] == 0 {
            continue;
        }
        let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
        let h = mask_hide(m, map, egui::pos2(region.min.x + x / scale, region.min.y + y / scale));
        let a = (255.0 * (1.0 - h)).round() as u16;
        if a == 255 {
            continue;
        }
        for c in px.iter_mut() {
            *c = ((*c as u16 * a + 127) / 255) as u8;
        }
    }
}

/// Rows `y0 .. y0 + h` of the render of `region` (output pixels): the same
/// transform as the whole picture moved up by whole pixels, so bands put
/// together match a single render exactly.
#[allow(clippy::too_many_arguments)]
fn render_band(doc: &Document, region: Rect, scale: f32, bg: Option<Color32>, y0: u32, h: u32, flat: &FlatCache) -> Result<Pixmap, String> {
    let (w, _) = output_size(region, scale);
    let full = region;
    // the band's own world rect (a pixel of margin for culling and source windows)
    let region = Rect::from_min_max(
        egui::pos2(full.min.x, full.min.y + y0 as f32 / scale),
        egui::pos2(full.max.x, (full.min.y + (y0 + h) as f32 / scale).min(full.max.y)),
    );
    let cull = region.expand(2.0 / scale);
    let dy = y0 as f32;
    let mut out = Pixmap::new(w, h).ok_or("이미지 버퍼를 만들 수 없음")?;
    if let Some(bg) = bg {
        let [r, g, b, a] = bg.to_srgba_unmultiplied();
        out.fill(tiny_skia::Color::from_rgba8(r, g, b, a));
    }
    let base = Transform::from_translate(-full.min.x, -full.min.y).post_scale(scale, scale).post_translate(0.0, -dy);
    let cams = crate::persp::cameras(&doc.spaces);
    for layer in doc.draw_order(&cams).into_iter().filter(|l| l.visible && l.opacity > 0.0) {
        let timed = crate::anim::layer_at_time(doc, layer);
        let layer = timed.as_ref().unwrap_or(layer);
        if layer.mask_of.is_some() || !layer.shown_bounds(&cams).is_some_and(|b| b.intersects(cull)) {
            continue;
        }
        if let Some(m) = mask_for(doc, layer.id) {
            let mut one = layer.clone();
            one.mask_of = None;
            let mini = Document { layers: vec![one], spaces: doc.spaces.clone(), timeline: doc.timeline.clone(), ..Default::default() };
            let mut pm = render_band(&mini, full, scale, None, y0, h, flat)?;
            apply_mask(&mut pm, doc, m, region, scale);
            out.draw_pixmap(0, 0, pm.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            continue;
        }
        if let Some((mode, op)) = crate::canvas::group_of(layer) {
            // drawn alone at full strength, then put down once (as on the canvas)
            let mut one = layer.clone();
            one.opacity = 1.0;
            match &mut one.kind {
                LayerKind::Text(t) => t.blend = Blend::Normal,
                LayerKind::Raster(r) => r.blend = Blend::Normal,
                _ => {}
            }
            let mini = Document { layers: vec![one], spaces: doc.spaces.clone(), timeline: doc.timeline.clone(), ..Default::default() };
            let tmp = render_band(&mini, full, scale, None, y0, h, flat)?;
            blend_onto(&mut out, &tmp, mode, op, bg);
            continue;
        }
        if let Some((pl, cam)) = layer.place.as_ref().and_then(|pl| cams.get(&pl.space).map(|c| (pl, c))) {
            // content -> canvas -> output pixels
            let s = scale as f64;
            let out_m = [[s, 0.0, -full.min.x as f64 * s], [0.0, s, -full.min.y as f64 * s - dy as f64], [0.0, 0.0, 1.0]];
            let h = crate::persp::mul_mm(&out_m, &pl.homography(cam));
            let res = magnification(&h, layer.bounds().unwrap_or(region)).clamp(0.05, 4.0);
            let clip = doc.spaces.iter().find(|s| s.id == pl.space).map(|sp| {
                let m = sp.clip();
                Rect::from_min_max(((m.min - full.min) * scale).to_pos2(), ((m.max - full.min) * scale).to_pos2()).translate(egui::vec2(0.0, -dy))
            });
            let light = doc.spaces.iter().find(|s| s.id == pl.space).and_then(|s| s.light).filter(|_| pl.thick > 0.0);
            // flattened once per export and shared by every band
            if let Some(fl) = flat.get(layer, pl, res) {
                let (src, rect, res) = (&fl.0, fl.1, fl.2);
                let to_out = |w: crate::persp::V3| cam.project(w).map(|p| egui::pos2((p.x - full.min.x) * scale, (p.y - full.min.y) * scale - dy));
                if pl.thick <= 0.0 {
                    warp_onto(&mut out, src, rect, res, &h, layer.opacity, clip, 1.0);
                } else {
                    let to_eye = |w: crate::persp::V3| crate::persp::sub(cam.center, w);
                    if let Some(edges) = &fl.3 {
                        let sides = crate::extrude::sides(edges, pl, &to_out, &to_eye, &|w| cam.to_cam(w)[2], light.as_ref(), crate::canvas::SIDE_SHADE);
                        fill_sides(&mut out, &sides, layer.opacity, clip);
                    }
                    let (off, n) = crate::extrude::visible_cap(pl, &to_eye);
                    let hc = if off == 0.0 { h } else { crate::persp::mul_mm(&out_m, &pl.homography_at(cam, off)) };
                    warp_onto(&mut out, src, rect, res, &hc, layer.opacity, clip, light.map_or(1.0, |l| l.shade(n)));
                }
            }
            continue;
        }
        if let LayerKind::Text(t) = &layer.kind {
            if t.blend != Blend::Normal {
                let mut tmp = Pixmap::new(w, h).ok_or("임시 버퍼를 만들 수 없음")?;
                draw_layer(&mut tmp, layer, base, cull, scale);
                blend_onto(&mut out, &tmp, t.blend, layer.opacity, bg);
                continue;
            }
        }
        if layer.opacity < 0.999 {
            let mut tmp = Pixmap::new(w, h).ok_or("임시 버퍼를 만들 수 없음")?;
            draw_layer(&mut tmp, layer, base, cull, scale);
            let paint = PixmapPaint { opacity: layer.opacity, ..Default::default() };
            out.draw_pixmap(0, 0, tmp.as_ref(), &paint, Transform::identity(), None);
        } else {
            draw_layer(&mut out, layer, base, cull, scale);
        }
    }
    Ok(out)
}

/// Flattened 3D layers (and their extrusion outlines) for one export, shared by
/// its bands.
#[derive(Default)]
pub struct FlatCache {
    #[allow(clippy::type_complexity)]
    map: std::sync::Mutex<HashMap<(u64, u32), std::sync::Arc<(Pixmap, Rect, f32, Option<Vec<crate::extrude::Edge>>)>>>,
}

impl FlatCache {
    #[allow(clippy::type_complexity)]
    fn get(&self, layer: &Layer, pl: &crate::persp::Placement, res: f32) -> Option<std::sync::Arc<(Pixmap, Rect, f32, Option<Vec<crate::extrude::Edge>>)>> {
        let key = (layer.id, res.to_bits());
        let mut m = self.map.lock().unwrap();
        if let Some(v) = m.get(&key) {
            return Some(v.clone());
        }
        let (src, rect, res) = flatten_layer(layer, res)?;
        let segs = pl.segs.clamp(1, 4);
        let edges = (pl.thick > 0.0).then(|| {
            if segs > 1 {
                // the outline from a finer picture (more segments)
                flatten_layer(layer, res * segs as f32).map_or_else(|| crate::extrude::trace(&src, rect, res), |(f, fr, fres)| crate::extrude::trace(&f, fr, fres))
            } else {
                crate::extrude::trace(&src, rect, res)
            }
        });
        let v = std::sync::Arc::new((src, rect, res, edges));
        m.insert(key, v.clone());
        Some(v)
    }
}

/// Extrusion side quads (output pixels, far to near) filled with their colours.
/// Each quad is stretched a little along its edge so neighbours overlap instead
/// of leaving anti-aliasing seams.
fn fill_sides(out: &mut Pixmap, sides: &[crate::extrude::Side], opacity: f32, clip: Option<Rect>) {
    let mask = clip.and_then(|c| {
        let mut m = tiny_skia::Mask::new(out.width(), out.height())?;
        let r = tiny_skia::Rect::from_ltrb(c.min.x, c.min.y, c.max.x, c.max.y)?;
        m.fill_path(&PathBuilder::from_rect(r), FillRule::Winding, false, Transform::identity());
        Some(m)
    });
    for sd in sides {
        let [a, b, c, d] = sd.q;
        let grow = |p: Pos2, q: Pos2| {
            let v = q - p;
            let l = v.length();
            if l > 1e-3 { p - v / l * 0.35 } else { p }
        };
        let (a, d) = (grow(a, b), grow(d, c));
        let (b, c) = (grow(b, a), grow(c, d));
        let mut pb = PathBuilder::new();
        pb.move_to(a.x, a.y);
        pb.line_to(b.x, b.y);
        pb.line_to(c.x, c.y);
        pb.line_to(d.x, d.y);
        pb.close();
        let Some(path) = pb.finish() else { continue };
        let to8 = |c: [f32; 3]| c.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8);
        let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut paint = Paint::default();
        let (ca, cb) = (to8(sd.ca), to8(sd.cb));
        // colours (and smooth shading) change along the edge: a gradient from end a
        // to end b, as the canvas mesh interpolates them
        let (ma, mb) = (sd.q[0].lerp(sd.q[3], 0.5), sd.q[1].lerp(sd.q[2], 0.5));
        let grad = (ca != cb && ma.distance(mb) > 0.5)
            .then(|| {
                tiny_skia::LinearGradient::new(
                    tiny_skia::Point::from_xy(ma.x, ma.y),
                    tiny_skia::Point::from_xy(mb.x, mb.y),
                    vec![
                        tiny_skia::GradientStop::new(0.0, tiny_skia::Color::from_rgba8(ca[0], ca[1], ca[2], alpha)),
                        tiny_skia::GradientStop::new(1.0, tiny_skia::Color::from_rgba8(cb[0], cb[1], cb[2], alpha)),
                    ],
                    tiny_skia::SpreadMode::Pad,
                    Transform::identity(),
                )
            })
            .flatten();
        match grad {
            Some(g) => paint.shader = g,
            None => {
                let rgb = [0, 1, 2].map(|k| ((ca[k] as u16 + cb[k] as u16) / 2) as u8);
                paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], alpha);
            }
        }
        paint.anti_alias = true;
        out.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), mask.as_ref());
    }
}

/// The picture of `region` at `scale`. Large renders are cut into horizontal
/// bands drawn on every core (a queue of a few bands per core, so empty bands
/// do not leave cores idle); the result is the same as one pass.
pub fn render(doc: &Document, region: Rect, scale: f32, bg: Option<Color32>) -> Result<Pixmap, String> {
    let (w, h) = output_size(region, scale);
    if w > MAX_SIDE || h > MAX_SIDE || w as u64 * h as u64 > MAX_PIXELS {
        return Err(format!("너무 큼: {w}×{h}px"));
    }
    let flat = FlatCache::default();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    if threads == 1 || (w as u64 * h as u64) < 600_000 {
        return render_band(doc, region, scale, bg, 0, h, &flat);
    }
    let n = (threads * 3).min(h as usize).max(1);
    let bh = h.div_ceil(n as u32);
    let ss = SS_K.with(|c| c.get());
    let next = std::sync::atomic::AtomicU32::new(0);
    let bands: std::sync::Mutex<Vec<(u32, Result<Pixmap, String>)>> = Default::default();
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| {
                SS_K.with(|c| c.set(ss));
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let y0 = i * bh;
                    if y0 >= h {
                        break;
                    }
                    let r = render_band(doc, region, scale, bg, y0, bh.min(h - y0), &flat);
                    bands.lock().unwrap().push((y0, r));
                }
            });
        }
    });
    let mut out = Pixmap::new(w, h).ok_or("이미지 버퍼를 만들 수 없음")?;
    let stride = w as usize * 4;
    for (y0, r) in bands.into_inner().unwrap() {
        let pm = r?;
        let at = y0 as usize * stride;
        out.data_mut()[at..at + pm.data().len()].copy_from_slice(pm.data());
    }
    Ok(out)
}

thread_local! {
    /// The supersampling factor of the render in progress on this thread (1: none).
    static SS_K: std::cell::Cell<f32> = const { std::cell::Cell::new(1.0) };
}

fn srgb_to_lin(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn lin_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// `render_rotated` drawn `k` times larger and averaged back down in linear light (k = 1..4,
/// lowered when the large image would not fit). Smooths edges and the slice
/// lines of thick 3D layers.
pub fn render_ss(doc: &Document, center: Pos2, size: egui::Vec2, rot: f32, scale: f32, bg: Option<Color32>, k: u8) -> Result<Pixmap, String> {
    let mut k = k.clamp(1, 4) as u32;
    while k > 1 {
        let (w, h) = ((size.x * scale * k as f32).ceil() as u64, (size.y * scale * k as f32).ceil() as u64);
        if w <= MAX_SIDE as u64 && h <= MAX_SIDE as u64 && w * h <= MAX_PIXELS / 2 {
            break;
        }
        k -= 1;
    }
    if k == 1 {
        return render_rotated(doc, center, size, rot, scale, bg);
    }
    SS_K.with(|c| c.set(k as f32));
    let big = render_rotated(doc, center, size, rot, scale * k as f32, bg);
    SS_K.with(|c| c.set(1.0));
    let big = big?;
    let (w, h) = (big.width() / k, big.height() / k);
    let mut out = Pixmap::new(w.max(1), h.max(1)).ok_or("이미지 버퍼를 만들 수 없음")?;
    let (bw, src) = (big.width() as usize, big.data());
    // average in linear light (premultiplied), then back to sRGB
    let lut: Vec<f32> = (0..256).map(|i| srgb_to_lin(i as f32 / 255.0)).collect();
    let n = (k * k) as f32;
    let row_bytes = w as usize * 4;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows_per = (h as usize).div_ceil(threads).max(1);
    let k = k as usize;
    std::thread::scope(|sc| {
        for (ci, chunk) in out.data_mut().chunks_mut(rows_per * row_bytes).enumerate() {
            let lut = &lut;
            sc.spawn(move || {
                let y_start = ci * rows_per;
                for (ry, dst_row) in chunk.chunks_mut(row_bytes).enumerate() {
                    let y = y_start + ry;
                    for x in 0..w as usize {
                        let mut acc = [0f32; 4];
                        for dy in 0..k {
                            let row = (y * k + dy) * bw;
                            for dx in 0..k {
                                let i = (row + x * k + dx) * 4;
                                let a = src[i + 3];
                                if a == 0 {
                                    continue;
                                }
                                let af = a as f32 / 255.0;
                                for c in 0..3 {
                                    // un-premultiply, to linear, premultiply again
                                    let v = ((src[i + c] as f32 / af).round().min(255.0)) as usize;
                                    acc[c] += lut[v] * af;
                                }
                                acc[3] += af;
                            }
                        }
                        let o = x * 4;
                        let a = acc[3] / n;
                        if a <= 0.0 {
                            dst_row[o..o + 4].copy_from_slice(&[0; 4]);
                            continue;
                        }
                        for c in 0..3 {
                            let lin = (acc[c] / n) / a;
                            dst_row[o + c] = ((lin_to_srgb(lin.clamp(0.0, 1.0)) * a * 255.0).round().min(a * 255.0)) as u8;
                        }
                        dst_row[o + 3] = (a * 255.0).round() as u8;
                    }
                }
            });
        }
    });
    Ok(out)
}

/// A rotated region (centre, size in world pixels, rotation clockwise) at `scale`:
/// its bounding box is rendered, then turned back into an upright image.
pub fn render_rotated(doc: &Document, center: Pos2, size: egui::Vec2, rot: f32, scale: f32, bg: Option<Color32>) -> Result<Pixmap, String> {
    if rot.abs() < 1e-6 {
        return render(doc, Rect::from_center_size(center, size), scale, bg);
    }
    let (s, c) = rot.sin_cos();
    let h = size * 0.5;
    let ext = vec2((h.x * c).abs() + (h.y * s).abs(), (h.x * s).abs() + (h.y * c).abs());
    let bb = Rect::from_center_size(center, ext * 2.0 + vec2(4.0, 4.0) / scale);
    let src = render(doc, bb, scale, bg)?;
    let (w, hh) = ((size.x * scale).round().max(1.0) as u32, (size.y * scale).round().max(1.0) as u32);
    let mut out = Pixmap::new(w, hh).ok_or("이미지 버퍼를 만들 수 없음")?;
    let ts = Transform::from_translate((bb.min.x - center.x) * scale, (bb.min.y - center.y) * scale)
        .post_rotate(-rot.to_degrees())
        .post_translate(w as f32 * 0.5, hh as f32 * 0.5);
    let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..Default::default() };
    out.draw_pixmap(0, 0, src.as_ref(), &paint, ts, None);
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
        LayerKind::Audio(_) => {}
        LayerKind::Image(im) => draw_image(pm, im, base, scale, region),
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
            // through `base`, so a band (shifted by whole pixels) lands the same
            let o = vec2(base.sx * t.pos.x + base.kx * t.pos.y + base.tx, base.ky * t.pos.x + base.sy * t.pos.y + base.ty);
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
#[allow(clippy::too_many_arguments)]
pub fn warp_onto(dst: &mut Pixmap, src: &Pixmap, rect: Rect, res: f32, h: &crate::persp::M3, opacity: f32, clip: Option<Rect>, shade: f32) {
    use crate::persp::{apply_h, invert};
    let Some(hi) = invert(h) else { return };
    let Some(q) = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()]
        .iter()
        .map(|p| apply_h(h, *p))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let mut bb = Rect::from_points(&q);
    if let Some(c) = clip {
        bb = bb.intersect(c);
        if !bb.is_positive() {
            return;
        }
    }
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
            for v in &mut s[..3] {
                *v *= shade;
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
    // preview frames (video) are smaller than the logical size the crop is in
    let k = iw as f32 / im.src.size().x.max(1.0);
    let x0 = ((im.crop.min.x * k).round() as i32).clamp(0, iw - 1);
    let y0 = ((im.crop.min.y * k).round() as i32).clamp(0, ih - 1);
    let x1 = ((im.crop.max.x * k).round() as i32).clamp(x0 + 1, iw);
    let y1 = ((im.crop.max.y * k).round() as i32).clamp(y0 + 1, ih);
    let mut sub = image::imageops::crop_imm(&*im.src.rgba, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32).to_image();
    let mut factor = 1.0 / k;
    let target_scale = target_scale / k;
    if target_scale < 0.5 {
        let nw = ((sub.width() as f32 * target_scale * 1.5).round() as u32).max(1);
        let nh = ((sub.height() as f32 * target_scale * 1.5).round() as u32).max(1);
        factor *= sub.width() as f32 / nw as f32;
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

fn draw_image(pm: &mut Pixmap, im: &ImageLayer, base: Transform, scale: f32, region: Rect) {
    // only the source pixels the region needs (small regions: eyedropper, magnifier)
    let win = Rect::from_points(&[region.left_top(), region.right_top(), region.right_bottom(), region.left_bottom()].map(|p| im.world_to_src(p)))
        .expand(3.0)
        .intersect(im.crop);
    if !win.is_positive() {
        return;
    }
    let mut sub = im.clone();
    sub.crop = Rect::from_min_max(win.min.floor(), win.max.ceil()).intersect(im.crop);
    sub.center = im.src_to_world(sub.crop.center());
    let im = &sub;
    let Some((src, factor)) = image_pixmap(im, im.scale * scale) else { return };
    let (cw, ch) = (src.width() as f32 * factor, src.height() as f32 * factor);
    let (sin, cos) = im.rotation.sin_cos();
    let s = im.scale;
    // local (0..cw, 0..ch) -> world
    let place = Transform::from_row(cos * s, sin * s, -sin * s, cos * s, im.center.x, im.center.y)
        .pre_translate(-cw / 2.0, -ch / 2.0)
        .pre_scale(factor, factor);
    let ts = place.post_concat(base);
    // pixel for pixel (also 1:1 at the target of a supersampled render: each source
    // pixel becomes a k×k block that averages back to itself)
    let ss = SS_K.with(|c| c.get());
    let quality = if (im.scale * scale / ss - 1.0).abs() < 1e-4 && im.rotation == 0.0 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{ShapeKind, ShapeLayer};
    use egui::pos2;

    /// A camera turned 90° clockwise looks "up" along world +x, so the red right half
    /// of the scene comes out at the top of the image.
    #[test]
    fn rotated_region() {
        let mut doc = Document::default();
        doc.insert("r".into(), LayerKind::Shape(ShapeLayer { kind: ShapeKind::Rect, a: pos2(0.0, -50.0), b: pos2(50.0, 50.0), color: Color32::RED, width: 0.0, fill: Some(Color32::RED), rotation: 0.0 }), None);
        let pm = render_rotated(&doc, pos2(0.0, 0.0), vec2(100.0, 60.0), std::f32::consts::FRAC_PI_2, 1.0, Some(Color32::WHITE)).unwrap();
        assert_eq!((pm.width(), pm.height()), (100, 60));
        let top = pm.pixel(50, 10).unwrap().demultiply();
        let bottom = pm.pixel(50, 50).unwrap().demultiply();
        assert!(top.red() > 200 && top.green() < 60, "top {top:?}");
        assert!(bottom.green() > 200, "bottom {bottom:?}");
    }
}

#[cfg(test)]
mod ss_tests {
    use super::*;

    #[test]
    fn supersampling_keeps_the_size() {
        let mut doc = Document::default();
        let s = crate::doc::ShapeLayer { kind: crate::doc::ShapeKind::Rect, a: egui::pos2(10.0, 10.0), b: egui::pos2(90.0, 60.0), color: Color32::RED, width: 3.0, fill: Some(Color32::BLUE), rotation: 0.3 };
        doc.insert("r".into(), LayerKind::Shape(s), None);
        let one = render_ss(&doc, egui::pos2(50.0, 40.0), egui::vec2(100.0, 80.0), 0.0, 1.0, Some(Color32::WHITE), 1).unwrap();
        let three = render_ss(&doc, egui::pos2(50.0, 40.0), egui::vec2(100.0, 80.0), 0.0, 1.0, Some(Color32::WHITE), 3).unwrap();
        assert_eq!((one.width(), one.height()), (three.width(), three.height()));
        // the fill shows in both
        let px = |p: &Pixmap| p.pixel(50, 40).unwrap();
        assert!(px(&three).blue() > 200 && px(&one).blue() > 200);
    }

    #[test]
    fn masks_invert_scale_and_follow() {
        use crate::doc::{ShapeKind, ShapeLayer};
        use egui::pos2;
        let mut doc = Document::default();
        let red = ShapeLayer { kind: ShapeKind::Rect, a: pos2(0.0, 0.0), b: pos2(100.0, 100.0), color: Color32::RED, width: 0.0, fill: Some(Color32::RED), rotation: 0.0 };
        let owner = doc.insert("r".into(), LayerKind::Shape(red), None);
        let mut r = crate::raster::RasterLayer::default();
        r.fill_rect(Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(50.0, 100.0)), true);
        let mid = doc.insert("m".into(), LayerKind::Raster(r), None);
        let k0 = crate::anim::key_of(doc.get(owner).unwrap(), 0.0);
        {
            let m = doc.get_mut(mid).unwrap();
            m.mask_of = Some(owner);
            m.mask_anchor = Some((owner, k0.center));
            m.mask_pose = Some([k0.rot, k0.ext[0]]);
        }
        let m = doc.get(mid).unwrap().clone();
        // painted half hidden; inverted: the other half; half strength: half hidden
        assert_eq!(mask_hide(&m, None, pos2(25.0, 50.0)), 1.0);
        assert_eq!(mask_hide(&m, None, pos2(75.0, 50.0)), 0.0);
        let mut inv = m.clone();
        inv.mask_invert = true;
        assert_eq!(mask_hide(&inv, None, pos2(75.0, 50.0)), 1.0);
        let mut half = m.clone();
        half.opacity = 0.5;
        assert_eq!(mask_hide(&half, None, pos2(25.0, 50.0)), 0.5);
        // the owner turns half a turn about its centre: the hidden half swaps sides
        assert!(mask_map(&doc, &m).is_none(), "in step");
        if let Some(LayerKind::Shape(s)) = doc.get_mut(owner).map(|l| &mut l.kind) {
            s.rotation = std::f32::consts::PI;
        }
        let map = mask_map(&doc, &m).expect("follows");
        assert!(map.inv(map.fwd(pos2(10.0, 20.0))).distance(pos2(10.0, 20.0)) < 1e-3);
        assert_eq!(mask_hide(&m, Some(map), pos2(75.0, 50.0)), 1.0);
        assert_eq!(mask_hide(&m, Some(map), pos2(25.0, 50.0)), 0.0);
    }

    #[test]
    fn brush_layers_blend() {
        let mut doc = Document::default();
        let red = crate::doc::ShapeLayer { kind: crate::doc::ShapeKind::Rect, a: egui::pos2(0.0, 0.0), b: egui::pos2(40.0, 40.0), color: Color32::RED, width: 0.0, fill: Some(Color32::RED), rotation: 0.0 };
        doc.insert("r".into(), LayerKind::Shape(red), None);
        let mut r = crate::raster::RasterLayer { blend: Blend::Multiply, ..Default::default() };
        r.fill_rect(Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(40.0, 40.0)), true);
        doc.insert("b".into(), LayerKind::Raster(r), None);
        let region = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(40.0, 40.0));
        let pm = render(&doc, region, 1.0, Some(Color32::WHITE)).unwrap();
        // white multiplied onto red leaves red (a normal brush would paint it white)
        let p = pm.pixel(20, 20).unwrap();
        assert!(p.red() > 240 && p.green() < 10, "{p:?}");
    }

    #[test]
    fn bands_match_a_single_pass() {
        let mut doc = Document::default();
        for i in 0..12 {
            let f = i as f32;
            let sh = crate::doc::ShapeLayer { kind: if i % 2 == 0 { crate::doc::ShapeKind::Ellipse } else { crate::doc::ShapeKind::Rect }, a: egui::pos2(30.0 + f * 70.0, 20.0 + f * 55.0), b: egui::pos2(160.0 + f * 60.0, 140.0 + f * 50.0), color: Color32::from_rgb(200, (f * 20.0) as u8, 90), width: 4.5, fill: Some(Color32::from_rgba_unmultiplied(40, 120, 220, 150)), rotation: 0.17 * f };
            doc.insert(format!("s{i}"), LayerKind::Shape(sh), None);
        }
        let region = Rect::from_min_size(egui::pos2(0.3, 0.7), egui::vec2(1000.0, 800.0));
        let one = render_band(&doc, region, 1.0, Some(Color32::WHITE), 0, output_size(region, 1.0).1, &FlatCache::default()).unwrap();
        let par = render(&doc, region, 1.0, Some(Color32::WHITE)).unwrap();
        assert_eq!((one.width(), one.height()), (par.width(), par.height()));
        // tiny-skia's edge coverage depends a little on where a path is clipped, so
        // edge pixels may differ by a few steps; nothing else (no seams, no shifts)
        let n = one.data().len() / 4;
        let mut differing = 0usize;
        let mut sum = 0u64;
        for (a, b) in one.data().chunks(4).zip(par.data().chunks(4)) {
            let d = a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs()).max().unwrap();
            sum += d as u64;
            if d > 1 {
                differing += 1;
            }
        }
        assert!(differing * 100 < n, "{differing} of {n} pixels differ");
        assert!((sum as f64 / n as f64) < 0.3, "mean difference {}", sum as f64 / n as f64);
    }

    #[test]
    #[ignore]
    fn band_speed() {
        let mut doc = Document::default();
        for i in 0..60 {
            let f = i as f32;
            let sh = crate::doc::ShapeLayer { kind: crate::doc::ShapeKind::Ellipse, a: egui::pos2(f * 60.0, f * 40.0), b: egui::pos2(f * 60.0 + 900.0, f * 40.0 + 700.0), color: Color32::RED, width: 6.0, fill: Some(Color32::from_rgba_unmultiplied(40, 120, 220, 120)), rotation: 0.1 * f };
            doc.insert(format!("s{i}"), LayerKind::Shape(sh), None);
        }
        let region = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(3840.0, 2160.0));
        let t = std::time::Instant::now();
        let _ = render_band(&doc, region, 1.0, Some(Color32::WHITE), 0, 2160, &FlatCache::default()).unwrap();
        let one = t.elapsed();
        let t = std::time::Instant::now();
        let _ = render(&doc, region, 1.0, Some(Color32::WHITE)).unwrap();
        eprintln!("single {:?} bands {:?} threads {:?}", one, t.elapsed(), std::thread::available_parallelism());
    }

    #[test]
    fn photos_stay_pixel_exact_when_supersampled() {
        // a noisy 40x30 photo placed 1:1 on whole pixels
        let mut img = image::RgbaImage::new(40, 30);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 37 % 256) as u8, (y * 91 % 256) as u8, ((x * y) * 13 % 256) as u8, 255]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let src = crate::io::decode_image(png, Some("png")).unwrap();
        let im = crate::doc::ImageLayer { src, center: egui::pos2(20.0, 15.0), scale: 1.0, rotation: 0.0, crop: Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(40.0, 30.0)), adjust: Default::default(), video: None };
        let mut doc = Document::default();
        doc.insert("p".into(), LayerKind::Image(im), None);
        let one = render_ss(&doc, egui::pos2(20.0, 15.0), egui::vec2(40.0, 30.0), 0.0, 1.0, None, 1).unwrap();
        let four = render_ss(&doc, egui::pos2(20.0, 15.0), egui::vec2(40.0, 30.0), 0.0, 1.0, None, 4).unwrap();
        let worst = one.data().iter().zip(four.data()).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
        assert!(worst <= 1, "max channel difference {worst}");
    }
}
