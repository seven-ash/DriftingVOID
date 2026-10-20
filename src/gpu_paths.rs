//! 2D layers on the GPU (render overhaul, docs/updates/wgpu-renderer.md
//! section 9, item 2): text and shapes become paths of line edges, filled on
//! the GPU with the exact area coverage a CPU rasterizer accumulates
//! (`paths.wgsl`). Glyph outlines are kept in font units, once per font and
//! glyph, so text at any size, zoom or animated scale needs no rasterizing on
//! the CPU. Shapes are outlined (strokes by tiny-skia's stroker) per drawing,
//! in target pixels. Pen strokes (overlapping capsules) stay on the CPU.

use std::collections::HashMap;

use ab_glyph::{Font, FontArc, GlyphId, OutlineCurve, ScaleFont};
use egui::{Rect, vec2};
use tiny_skia::{LineCap, LineJoin, PathBuilder, Stroke, Transform};

use crate::doc::{Layer, LayerKind, ShapeKind, TextLayer};

/// Edges of many paths, each listed in the horizontal bands it crosses.
#[derive(Default)]
pub struct PathSet {
    pub edges: Vec<[f32; 4]>,
    pub bands: Vec<[u32; 2]>,
    pub idx: Vec<u32>,
}

/// Where one path is in a `PathSet`: its bounds (its own units), bands and edges.
#[derive(Clone, Copy)]
pub struct Rec {
    pub bbox: [f32; 4],
    pub band_off: u32,
    pub nb: u32,
    pub edge_off: u32,
    pub edge_cnt: u32,
}

impl PathSet {
    /// Adds a path's edges (x0, y0, x1, y1), in `nb` bands over its height.
    pub fn add(&mut self, edges: &[[f32; 4]], nb: u32) -> Option<Rec> {
        // edges flat in its own units add nothing upright, but do once turned:
        // kept in the list, left out of the bands
        if edges.iter().all(|e| e[1] == e[3]) {
            return None;
        }
        let mut bb = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for e in edges {
            bb = [bb[0].min(e[0]).min(e[2]), bb[1].min(e[1]).min(e[3]), bb[2].max(e[0]).max(e[2]), bb[3].max(e[1]).max(e[3])];
        }
        if bb[3] <= bb[1] {
            bb[3] = bb[1] + 1e-3;
        }
        let nb = nb.max(1);
        let bh = (bb[3] - bb[1]) / nb as f32;
        let edge_off = self.edges.len() as u32;
        // each edge in the bands its height crosses, a band more each way (the
        // shader's band of a pixel row may round to a neighbour)
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); nb as usize];
        for (i, e) in edges.iter().enumerate().filter(|(_, e)| e[1] != e[3]) {
            let band = |y: f32| ((y - bb[1]) / bh).floor().clamp(0.0, (nb - 1) as f32) as i64;
            let lo = (band(e[1].min(e[3])) - 1).max(0) as usize;
            let hi = (band(e[1].max(e[3])) + 1).min(nb as i64 - 1) as usize;
            for (b, l) in lists.iter_mut().enumerate().take(hi + 1).skip(lo) {
                let first = if b == lo { 0x8000_0000 } else { 0 };
                l.push((edge_off + i as u32) | first);
            }
        }
        let band_off = self.bands.len() as u32;
        for l in lists {
            self.bands.push([self.idx.len() as u32, l.len() as u32]);
            self.idx.extend(l);
        }
        self.edges.extend_from_slice(edges);
        Some(Rec { bbox: bb, band_off, nb, edge_off, edge_cnt: edges.len() as u32 })
    }
}

/// One path drawn: `Rec` placed by `m` (x' = m0 x + m2 y + t0, y' = m1 x +
/// m3 y + t1) in a premultiplied colour (`paths.wgsl` `In`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Inst {
    pub bbox: [f32; 4],
    pub m: [f32; 4],
    pub t: [f32; 4],
    pub col: [f32; 4],
    pub rec: [u32; 4],
}

pub const INST_BYTES: u64 = std::mem::size_of::<Inst>() as u64;

impl Inst {
    pub fn new(r: &Rec, m: [f32; 4], t: [f32; 2], col: [f32; 4]) -> Inst {
        Inst { bbox: r.bbox, m, t: [t[0], t[1], 0.0, 0.0], col, rec: [r.band_off, r.nb, r.edge_off, r.edge_cnt] }
    }
}

pub fn inst_bytes(v: &[Inst]) -> &[u8] {
    // plain f32 and u32 arrays, no padding
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

/// Bands for a path of `n` edges.
fn bands_for(n: usize) -> u32 {
    (n as u32 / 3).clamp(1, 64)
}

/// Line edges of a glyph's outline, font units (y up), flattened finely enough
/// for any size it is shown at.
fn glyph_edges(font: &FontArc, gid: GlyphId) -> Vec<[f32; 4]> {
    let Some(o) = font.outline(gid) else { return Vec::new() };
    let upem = font.units_per_em().unwrap_or(1000.0);
    // a quarter pixel at 1024 px per em
    let tol = upem / 4096.0;
    let mut out = Vec::new();
    for c in &o.curves {
        match *c {
            OutlineCurve::Line(a, b) => out.push([a.x, a.y, b.x, b.y]),
            OutlineCurve::Quad(a, b, c) => quad(&mut out, [a.x, a.y], [b.x, b.y], [c.x, c.y], tol),
            OutlineCurve::Cubic(a, b, c, d) => cubic(&mut out, [a.x, a.y], [b.x, b.y], [c.x, c.y], [d.x, d.y], tol, 0),
        }
    }
    out
}

fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn quad(out: &mut Vec<[f32; 4]>, a: [f32; 2], b: [f32; 2], c: [f32; 2], tol: f32) {
    let dd = ((a[0] - 2.0 * b[0] + c[0]).powi(2) + (a[1] - 2.0 * b[1] + c[1]).powi(2)).sqrt();
    let n = ((dd / (8.0 * tol)).sqrt().ceil() as usize).clamp(1, 64);
    let mut p = a;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let q = if i == n { c } else { lerp(lerp(a, b, t), lerp(b, c, t), t) };
        out.push([p[0], p[1], q[0], q[1]]);
        p = q;
    }
}

fn cubic(out: &mut Vec<[f32; 4]>, a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2], tol: f32, depth: u8) {
    let dist = |p: [f32; 2], q: [f32; 2]| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
    let long = dist(a, b) + dist(b, c) + dist(c, d);
    let short = dist(a, d);
    if depth >= 12 || long - short <= tol {
        out.push([a[0], a[1], d[0], d[1]]);
        return;
    }
    let (ab, bc, cd) = (lerp(a, b, 0.5), lerp(b, c, 0.5), lerp(c, d, 0.5));
    let (abc, bcd) = (lerp(ab, bc, 0.5), lerp(bc, cd, 0.5));
    let m = lerp(abc, bcd, 0.5);
    cubic(out, a, ab, abc, m, tol, depth + 1);
    cubic(out, m, bcd, cd, d, tol, depth + 1);
}

/// A tiny-skia path through `ts` as line edges (target pixels).
fn path_edges(path: &tiny_skia::Path, ts: Transform, out: &mut Vec<[f32; 4]>) {
    use tiny_skia::PathSegment;
    let map = |p: tiny_skia::Point| [ts.sx * p.x + ts.kx * p.y + ts.tx, ts.ky * p.x + ts.sy * p.y + ts.ty];
    let tol = 0.05;
    let (mut start, mut cur) = ([0.0f32; 2], [0.0f32; 2]);
    let close = |out: &mut Vec<[f32; 4]>, cur: [f32; 2], start: [f32; 2]| {
        if cur != start {
            out.push([cur[0], cur[1], start[0], start[1]]);
        }
    };
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                close(out, cur, start);
                start = map(p);
                cur = start;
            }
            PathSegment::LineTo(p) => {
                let q = map(p);
                out.push([cur[0], cur[1], q[0], q[1]]);
                cur = q;
            }
            PathSegment::QuadTo(b, c) => {
                let c = map(c);
                quad(out, cur, map(b), c, tol);
                cur = c;
            }
            PathSegment::CubicTo(b, c, d) => {
                let d = map(d);
                cubic(out, cur, map(b), map(c), d, tol, 0);
                cur = d;
            }
            PathSegment::Close => {
                close(out, cur, start);
                cur = start;
            }
        }
    }
    // fills close every contour
    close(out, cur, start);
}

fn premul(c: egui::Color32) -> [f32; 4] {
    let [r, g, b, a] = c.to_srgba_unmultiplied().map(|v| v as f32 / 255.0);
    [r * a, g * a, b * a, a]
}

/// Glyph outlines kept across drawings, by font and glyph.
#[derive(Default)]
pub struct Glyphs {
    pub set: PathSet,
    recs: HashMap<(u64, u16), Option<Rec>>,
    /// Bumped when glyphs were added (the GPU copy is made again).
    pub rev: u64,
}

impl Glyphs {
    fn get(&mut self, fkey: u64, font: &FontArc, gid: GlyphId) -> Option<Rec> {
        if let Some(r) = self.recs.get(&(fkey, gid.0)) {
            return *r;
        }
        let e = glyph_edges(font, gid);
        let r = self.set.add(&e, bands_for(e.len()));
        self.recs.insert((fkey, gid.0), r);
        self.rev += 1;
        r
    }
}

fn font_key(t: &TextLayer) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&t.family, &t.style).hash(&mut h);
    h.finish()
}

/// What a 2D layer draws on the GPU: glyph instances (in `Glyphs`), and
/// paths of its own (shapes, a text's background) with their instances. None:
/// a kind drawn on the CPU.
pub struct Drawing {
    pub glyphs: Vec<Inst>,
    pub local: PathSet,
    pub shapes: Vec<Inst>,
}

/// Whether a layer is drawn here (text and shapes) rather than on the CPU.
pub fn handles(layer: &Layer) -> bool {
    matches!(layer.kind, LayerKind::Text(_) | LayerKind::Shape(_))
}

/// The layer as `export::draw_layer` draws it through `base` (canvas -> target
/// pixels) at `scale` target pixels per canvas unit.
pub fn drawing(glyphs: &mut Glyphs, layer: &Layer, base: Transform, scale: f32) -> Option<Drawing> {
    let mut d = Drawing { glyphs: Vec::new(), local: PathSet::default(), shapes: Vec::new() };
    let fill = |d: &mut Drawing, edges: &[[f32; 4]], col: [f32; 4]| {
        // in target pixels: a band per two rows at most
        let (lo, hi) = edges.iter().fold((f32::MAX, f32::MIN), |m, e| (m.0.min(e[1]).min(e[3]), m.1.max(e[1]).max(e[3])));
        let nb = bands_for(edges.len()).max(((hi - lo) / 2.0) as u32).min(256);
        if let Some(r) = d.local.add(edges, nb) {
            d.shapes.push(Inst::new(&r, [1.0, 0.0, 0.0, 1.0], [0.0, 0.0], col));
        }
    };
    match &layer.kind {
        LayerKind::Text(t) => {
            let lay = crate::text::layout(t);
            let o = vec2(base.sx * t.pos.x + base.kx * t.pos.y + base.tx, base.ky * t.pos.x + base.sy * t.pos.y + base.ty);
            // turned about the box's centre, as the CPU turns its picture
            let rot = if t.rotation == 0.0 { None } else { Some((t.rotation.sin_cos(), o + lay.size * scale * 0.5)) };
            let turn = |p: [f32; 2]| match rot {
                Some(((sn, cs), c)) => {
                    let (x, y) = (p[0] - c.x, p[1] - c.y);
                    [c.x + cs * x - sn * y, c.y + sn * x + cs * y]
                }
                None => p,
            };
            if let Some(bg) = t.bg {
                if let Some(path) = crate::text::rounded_rect(o.x, o.y, lay.size.x * scale, lay.size.y * scale, t.size * 0.3 * scale) {
                    let mut e = Vec::new();
                    path_edges(&path, Transform::identity(), &mut e);
                    let e: Vec<[f32; 4]> = e
                        .into_iter()
                        .map(|s| {
                            let (a, b) = (turn([s[0], s[1]]), turn([s[2], s[3]]));
                            [a[0], a[1], b[0], b[1]]
                        })
                        .collect();
                    fill(&mut d, &e, premul(bg));
                }
            }
            let font = crate::text::font_for(t);
            let fkey = font_key(t);
            let px = crate::text::em_scale(&font, (t.size * scale).max(0.5));
            let sf = font.as_scaled(px);
            let (hs, vs) = (sf.h_scale_factor(), sf.v_scale_factor());
            let [cr, cg, cb, ca] = t.color.to_srgba_unmultiplied().map(|v| v as f32 / 255.0);
            let col = [cr * ca, cg * ca, cb * ca, ca];
            // font units -> pixels: (x, y) -> pos + (x hs, −y vs), then turned
            let (m, turn_t) = match rot {
                Some(((sn, cs), _)) => ([cs * hs, sn * hs, sn * vs, -cs * vs], true),
                None => ([hs, 0.0, 0.0, -vs], false),
            };
            for &(gid, x, y) in &lay.glyphs {
                let Some(r) = glyphs.get(fkey, &font, gid) else { continue };
                let pos = [o.x + (lay.pad + x) * scale, o.y + (lay.pad + y) * scale];
                let t = if turn_t { turn(pos) } else { pos };
                d.glyphs.push(Inst::new(&r, m, t, col));
            }
        }
        LayerKind::Shape(s) => {
            let stroke = Stroke { width: s.width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
            let sp = premul(s.color);
            let outline = |path: &tiny_skia::Path, ts: Transform, out: &mut Vec<[f32; 4]>| {
                // the stroke's outline, in units fine enough for this scale
                let res = (ts.sx * ts.sx + ts.ky * ts.ky).sqrt().max(1e-3);
                if let Some(o) = path.stroke(&stroke, res) {
                    path_edges(&o, ts, out);
                }
            };
            match s.kind {
                ShapeKind::Line | ShapeKind::Arrow => {
                    let end = if s.kind == ShapeKind::Arrow { s.arrow_head().0 } else { s.b };
                    let mut pb = PathBuilder::new();
                    pb.move_to(s.a.x, s.a.y);
                    pb.line_to(end.x, end.y);
                    if let Some(path) = pb.finish() {
                        let mut e = Vec::new();
                        outline(&path, base, &mut e);
                        fill(&mut d, &e, sp);
                    }
                    if s.kind == ShapeKind::Arrow {
                        let (_, h) = s.arrow_head();
                        let mut pb = PathBuilder::new();
                        pb.move_to(h[0].x, h[0].y);
                        pb.line_to(h[1].x, h[1].y);
                        pb.line_to(h[2].x, h[2].y);
                        pb.close();
                        if let Some(path) = pb.finish() {
                            let mut e = Vec::new();
                            path_edges(&path, base, &mut e);
                            fill(&mut d, &e, sp);
                        }
                    }
                }
                ShapeKind::Rect | ShapeKind::Ellipse => {
                    let r = Rect::from_two_pos(s.a, s.b);
                    let tr = tiny_skia::Rect::from_ltrb(r.min.x, r.min.y, r.max.x.max(r.min.x + 0.01), r.max.y.max(r.min.y + 0.01))?;
                    let path = if s.kind == ShapeKind::Rect { Some(PathBuilder::from_rect(tr)) } else { PathBuilder::from_oval(tr) };
                    let c = r.center();
                    let ts = base.pre_concat(Transform::from_rotate_at(s.rotation.to_degrees(), c.x, c.y));
                    if let Some(path) = path {
                        if let Some(f) = s.fill {
                            let mut e = Vec::new();
                            path_edges(&path, ts, &mut e);
                            fill(&mut d, &e, premul(f));
                        }
                        if s.width > 0.0 {
                            let mut e = Vec::new();
                            outline(&path, ts, &mut e);
                            fill(&mut d, &e, sp);
                        }
                    }
                }
            }
        }
        _ => return None,
    }
    Some(d)
}
