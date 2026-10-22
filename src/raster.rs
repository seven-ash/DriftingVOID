//! Unbounded tiled raster layer and the pressure brush engine.
//! Pixels are premultiplied RGBA8 (same as tiny-skia), one pixel per world unit.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Pos2, Rect, Vec2, pos2};

use crate::doc::{TileMap, next_rev};

pub const TILE: i32 = 256;
const TILE_US: usize = TILE as usize;

#[derive(Clone)]
pub struct Tile {
    pub rev: u64,
    pub px: Vec<u8>,
}

impl Tile {
    pub fn empty() -> Self {
        Self { rev: next_rev(), px: vec![0; TILE_US * TILE_US * 4] }
    }

    pub fn is_empty(&self) -> bool {
        self.px.chunks_exact(4).all(|p| p[3] == 0)
    }
}

#[derive(Clone, Default)]
pub struct RasterLayer {
    /// Integer-valued world offset of local pixel (0,0).
    pub offset: Vec2,
    pub tiles: TileMap,
    /// Brush strokes recorded on the timeline (in drawing order) and the pixels from
    /// before the first of them; `tiles` stays the final picture.
    pub recs: Vec<Arc<BrushRec>>,
    pub rec_base: Option<TileMap>,
    /// How the layer mixes with what is under it.
    pub blend: crate::doc::Blend,
}

/// One recorded brush (or eraser) stroke: input points (local x, y, pressure) with
/// times relative to `start` on the timeline.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BrushRec {
    pub start: f64,
    pub params: BrushParams,
    pub pts: Vec<[f32; 3]>,
    pub times: Vec<f32>,
}

impl BrushRec {
    pub fn end(&self) -> f64 {
        self.start + self.times.last().copied().unwrap_or(0.0) as f64
    }
}

type ReplayKey = (usize, usize, u64, u32, u32);

/// Recent replays (paused playhead, canvas and export asking for the same time).
static REPLAYS: std::sync::Mutex<Vec<(ReplayKey, RasterLayer)>> = std::sync::Mutex::new(Vec::new());

impl RasterLayer {
    /// When the last recorded stroke finishes.
    pub fn rec_end(&self) -> Option<f64> {
        self.recs.iter().map(|r| r.end()).reduce(f64::max)
    }

    /// The layer as it looks at timeline time `t`: the pixels from before recording
    /// plus every recorded stroke drawn up to `t`. None when `tiles` already is it.
    pub fn replayed(&self, t: f64) -> Option<RasterLayer> {
        let base = self.rec_base.as_ref()?;
        if self.recs.is_empty() || self.rec_end().is_some_and(|e| t >= e) {
            return None;
        }
        let key = (Arc::as_ptr(self.recs.last().unwrap()) as usize, self.recs.len(), t.to_bits(), self.offset.x.to_bits(), self.offset.y.to_bits());
        if let Ok(c) = REPLAYS.lock() {
            if let Some((_, l)) = c.iter().find(|(k, _)| *k == key) {
                return Some(l.clone());
            }
        }
        let mut l = RasterLayer { offset: self.offset, tiles: base.clone(), blend: self.blend, ..Default::default() };
        for r in &self.recs {
            let local = (t - r.start) as f32;
            if local < 0.0 {
                continue;
            }
            let mut st = RasterStroke::new(r.params);
            for (p, tt) in r.pts.iter().zip(&r.times) {
                if *tt > local {
                    break;
                }
                st.add_point(&mut l, egui::pos2(p[0], p[1]), p[2]);
            }
            st.finish(&mut l);
        }
        if let Ok(mut c) = REPLAYS.lock() {
            c.push((key, l.clone()));
            if c.len() > 8 {
                c.remove(0);
            }
        }
        Some(l)
    }
}


pub fn tile_of(x: i32, y: i32) -> ((i32, i32), usize) {
    let tx = x.div_euclid(TILE);
    let ty = y.div_euclid(TILE);
    let lx = x.rem_euclid(TILE) as usize;
    let ly = y.rem_euclid(TILE) as usize;
    ((tx, ty), (ly * TILE_US + lx) * 4)
}

impl RasterLayer {
    /// Sets one world pixel's coverage (premultiplied white).
    pub fn set_alpha_world(&mut self, x: i32, y: i32, a: u8) {
        let (lx, ly) = (x - self.offset.x as i32, y - self.offset.y as i32);
        let (k, i) = tile_of(lx, ly);
        if a == 0 && !self.tiles.contains_key(&k) {
            return;
        }
        let t = Arc::make_mut(self.tiles.entry(k).or_insert_with(|| Arc::new(Tile::empty())));
        t.rev = next_rev();
        t.px[i..i + 4].copy_from_slice(&[a; 4]);
    }

    /// Sets (white, opaque) or clears every pixel of a world-space rect.
    pub fn fill_rect(&mut self, r: Rect, on: bool) {
        let x0 = (r.min.x - self.offset.x).floor() as i32;
        let y0 = (r.min.y - self.offset.y).floor() as i32;
        let x1 = (r.max.x - self.offset.x).ceil() as i32;
        let y1 = (r.max.y - self.offset.y).ceil() as i32;
        let v: [u8; 4] = if on { [255; 4] } else { [0; 4] };
        for ty in y0.div_euclid(TILE)..=(y1 - 1).div_euclid(TILE) {
            for tx in x0.div_euclid(TILE)..=(x1 - 1).div_euclid(TILE) {
                let key = (tx, ty);
                if !on && !self.tiles.contains_key(&key) {
                    continue;
                }
                let t = self.tiles.entry(key).or_insert_with(|| Arc::new(Tile::empty()));
                let t = Arc::make_mut(t);
                t.rev = next_rev();
                let (bx, by) = (tx * TILE, ty * TILE);
                for y in y0.max(by)..y1.min(by + TILE) {
                    for x in x0.max(bx)..x1.min(bx + TILE) {
                        let i = (((y - by) * TILE + (x - bx)) * 4) as usize;
                        t.px[i..i + 4].copy_from_slice(&v);
                    }
                }
            }
        }
    }

    pub fn pixel(&self, x: i32, y: i32) -> [u8; 4] {
        let (k, i) = tile_of(x, y);
        match self.tiles.get(&k) {
            Some(t) => [t.px[i], t.px[i + 1], t.px[i + 2], t.px[i + 3]],
            None => [0; 4],
        }
    }

    pub fn alpha_at_world(&self, p: Pos2) -> u8 {
        let l = p - self.offset;
        self.pixel(l.x.floor() as i32, l.y.floor() as i32)[3]
    }

    /// Exact bounds of non-transparent pixels, in world space. Each tile's
    /// own bounds are kept by its revision (`tile_bounds`), so this costs a
    /// look-up per tile, not a scan of every pixel (it runs several times a
    /// frame; Ash saw frame times grow with the tile count, 10-22).
    pub fn bounds(&self) -> Option<Rect> {
        let mut acc: Option<(i32, i32, i32, i32)> = None;
        for (&(tx, ty), t) in &self.tiles {
            if let Some([x0, y0, x1, y1]) = tile_bounds(t) {
                let r = (tx * TILE + x0, ty * TILE + y0, tx * TILE + x1 + 1, ty * TILE + y1 + 1);
                acc = Some(match acc {
                    None => r,
                    Some(a) => (a.0.min(r.0), a.1.min(r.1), a.2.max(r.2), a.3.max(r.3)),
                });
            }
        }
        acc.map(|(x0, y0, x1, y1)| {
            Rect::from_min_max(pos2(x0 as f32, y0 as f32), pos2(x1 as f32, y1 as f32))
                .translate(self.offset)
        })
    }

}

/// A tile's non-transparent pixels (first and last column and row), kept by
/// the tile's revision: every change to a tile's pixels gives it a new one.
fn tile_bounds(t: &Tile) -> Option<[i32; 4]> {
    type Cache = std::collections::HashMap<u64, Option<[i32; 4]>>;
    static CACHE: std::sync::Mutex<Option<Cache>> = std::sync::Mutex::new(None);
    if let Some(b) = CACHE.lock().ok().and_then(|c| c.as_ref().and_then(|c| c.get(&t.rev).copied())) {
        return b;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for y in 0..TILE {
        let row = &t.px[(y as usize * TILE_US) * 4..((y as usize + 1) * TILE_US) * 4];
        for x in 0..TILE {
            if row[x as usize * 4 + 3] != 0 {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    let b = (x0 <= x1).then_some([x0, y0, x1, y1]);
    if let Ok(mut c) = CACHE.lock() {
        let c = c.get_or_insert_with(Default::default);
        // revisions only grow; old ones are dropped wholesale now and then
        if c.len() > 200_000 {
            c.clear();
        }
        c.insert(t.rev, b);
    }
    b
}

impl RasterLayer {
    /// Copies the local pixel rectangle [x0,x1)x[y0,y1) into a premultiplied buffer.
    pub fn read_region(&self, x0: i32, y0: i32, w: i32, h: i32) -> Vec<u8> {
        let mut out = vec![0u8; (w * h * 4) as usize];
        let tx0 = x0.div_euclid(TILE);
        let ty0 = y0.div_euclid(TILE);
        let tx1 = (x0 + w - 1).div_euclid(TILE);
        let ty1 = (y0 + h - 1).div_euclid(TILE);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let Some(t) = self.tiles.get(&(tx, ty)) else { continue };
                let ox = tx * TILE;
                let oy = ty * TILE;
                let sx0 = x0.max(ox);
                let sx1 = (x0 + w).min(ox + TILE);
                let sy0 = y0.max(oy);
                let sy1 = (y0 + h).min(oy + TILE);
                if sx0 >= sx1 || sy0 >= sy1 {
                    continue;
                }
                let n = ((sx1 - sx0) * 4) as usize;
                for y in sy0..sy1 {
                    let si = (((y - oy) * TILE + (sx0 - ox)) * 4) as usize;
                    let di = (((y - y0) * w + (sx0 - x0)) * 4) as usize;
                    out[di..di + n].copy_from_slice(&t.px[si..si + n]);
                }
            }
        }
        out
    }

}

impl RasterLayer {
    /// Splits off the pixels whose centres lie inside the given world polygons
    /// (even-odd rule): returns (patch, rest), both with this layer's offset.
    pub fn lift(&self, polys: &[Vec<Pos2>]) -> (RasterLayer, RasterLayer) {
        let mut patch = RasterLayer { offset: self.offset, ..Default::default() };
        let mut rest = self.clone();
        let mut touched = std::collections::HashSet::new();
        for poly in polys {
            if poly.len() < 3 {
                continue;
            }
            let lp: Vec<Pos2> = poly.iter().map(|p| *p - self.offset).collect();
            let b = Rect::from_points(&lp);
            let (y0, y1) = (b.min.y.floor() as i32, b.max.y.ceil() as i32);
            let mut xs: Vec<f32> = Vec::new();
            for y in y0..y1 {
                let yc = y as f32 + 0.5;
                xs.clear();
                for i in 0..lp.len() {
                    let (a, c) = (lp[i], lp[(i + 1) % lp.len()]);
                    if (a.y <= yc) != (c.y <= yc) {
                        xs.push(a.x + (yc - a.y) / (c.y - a.y) * (c.x - a.x));
                    }
                }
                xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                for pair in xs.chunks_exact(2) {
                    let xa = (pair[0] - 0.5).ceil() as i32;
                    let xb = (pair[1] - 0.5).floor() as i32;
                    for x in xa..=xb {
                        let (k, i) = tile_of(x, y);
                        let Some(src) = self.tiles.get(&k) else { continue };
                        if src.px[i + 3] == 0 {
                            continue;
                        }
                        let pt = Arc::make_mut(patch.tiles.entry(k).or_insert_with(|| Arc::new(Tile::empty())));
                        pt.px[i..i + 4].copy_from_slice(&src.px[i..i + 4]);
                        let rt = Arc::make_mut(rest.tiles.get_mut(&k).unwrap());
                        rt.px[i..i + 4].fill(0);
                        touched.insert(k);
                    }
                }
            }
        }
        for k in touched {
            if let Some(t) = patch.tiles.get_mut(&k) {
                Arc::make_mut(t).rev = next_rev();
            }
            if let Some(t) = rest.tiles.get_mut(&k) {
                Arc::make_mut(t).rev = next_rev();
            }
        }
        rest.tiles.retain(|_, t| !t.is_empty());
        (patch, rest)
    }

    /// Paints `top` over this layer (src-over). Offsets are whole pixels.
    pub fn composite_over(&mut self, top: &RasterLayer) {
        let d = top.offset - self.offset;
        let (dx, dy) = (d.x.round() as i32, d.y.round() as i32);
        let mut touched = std::collections::HashSet::new();
        for (&(tx, ty), t) in &top.tiles {
            for y in 0..TILE {
                for x in 0..TILE {
                    let si = ((y * TILE + x) * 4) as usize;
                    let sa = t.px[si + 3];
                    if sa == 0 {
                        continue;
                    }
                    let (k, i) = tile_of(tx * TILE + x + dx, ty * TILE + y + dy);
                    touched.insert(k);
                    let dt = Arc::make_mut(self.tiles.entry(k).or_insert_with(|| Arc::new(Tile::empty())));
                    let f = 1.0 - sa as f32 / 255.0;
                    for c in 0..4 {
                        dt.px[i + c] = (t.px[si + c] as f32 + dt.px[i + c] as f32 * f).round().min(255.0) as u8;
                    }
                }
            }
        }
        for k in touched {
            if let Some(t) = self.tiles.get_mut(&k) {
                Arc::make_mut(t).rev = next_rev();
            }
        }
    }
}

/// Resamples a raster layer scaled by `s` and rotated by `th` about world point `c`.
pub fn transformed(src: &RasterLayer, c: Pos2, s: f32, th: f32) -> Option<RasterLayer> {
    let b = src.bounds()?;
    let lb = b.translate(-src.offset);
    let (x0, y0) = (lb.min.x as i32, lb.min.y as i32);
    let (w, h) = (lb.width() as i32, lb.height() as i32);
    let pm = tiny_skia::Pixmap::from_vec(src.read_region(x0, y0, w, h), tiny_skia::IntSize::from_wh(w as u32, h as u32)?)?;
    let (sin, cos) = th.sin_cos();
    let map = |p: Pos2| {
        let d = (p - c) * s;
        c + Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
    };
    let corners = [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()].map(map);
    let nb = Rect::from_points(&corners);
    let (nx0, ny0) = (nb.min.x.floor(), nb.min.y.floor());
    let (nw, nh) = ((nb.max.x.ceil() - nx0) as u32 + 1, (nb.max.y.ceil() - ny0) as u32 + 1);
    if nw as u64 * nh as u64 > 200_000_000 {
        return None;
    }
    let mut out = tiny_skia::Pixmap::new(nw, nh)?;
    // pixmap pixel -> world (b.min + p) -> transformed world -> output pixel
    let ts = tiny_skia::Transform::from_translate(b.min.x - c.x, b.min.y - c.y)
        .post_scale(s, s)
        .post_concat(tiny_skia::Transform::from_row(cos, sin, -sin, cos, 0.0, 0.0))
        .post_translate(c.x - nx0, c.y - ny0);
    let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() };
    out.draw_pixmap(0, 0, pm.as_ref(), &paint, ts, None);
    let mut n = RasterLayer { offset: Vec2::new(nx0, ny0), blend: src.blend, ..Default::default() };
    let data = out.data();
    for y in 0..nh as i32 {
        for x in 0..nw as i32 {
            let i = ((y * nw as i32 + x) * 4) as usize;
            if data[i + 3] == 0 {
                continue;
            }
            let (k, ti) = tile_of(x, y);
            let t = Arc::make_mut(n.tiles.entry(k).or_insert_with(|| Arc::new(Tile::empty())));
            t.px[ti..ti + 4].copy_from_slice(&data[i..i + 4]);
        }
    }
    Some(n)
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct BrushParams {
    pub color: Color32,
    /// Radius in world pixels at full pressure.
    pub radius: f32,
    pub opacity: f32,
    pub hardness: f32,
    pub pressure_size: bool,
    pub pressure_opacity: bool,
    pub erase: bool,
}

/// One brush stroke in "wash" mode: dabs accumulate into a coverage mask with
/// max(), and the layer is recomposited from the pre-stroke pixels, so overlapping
/// dabs within a stroke never build up past the chosen opacity.
pub struct RasterStroke {
    pub p: BrushParams,
    mask: HashMap<(i32, i32), Vec<f32>>,
    orig: HashMap<(i32, i32), Option<Arc<Tile>>>,
    last: Option<(Pos2, f32)>,
    carry: f32,
}

impl RasterStroke {
    pub fn new(p: BrushParams) -> Self {
        Self { p, mask: HashMap::new(), orig: HashMap::new(), last: None, carry: 0.0 }
    }

    fn radius_at(&self, pressure: f32) -> f32 {
        let r = if self.p.pressure_size { self.p.radius * (0.12 + 0.88 * pressure) } else { self.p.radius };
        r.max(0.5)
    }

    fn alpha_at(&self, pressure: f32) -> f32 {
        if self.p.pressure_opacity { pressure.clamp(0.0, 1.0) } else { 1.0 }
    }

    /// `p` is in the layer's local pixel space.
    pub fn add_point(&mut self, layer: &mut RasterLayer, p: Pos2, pressure: f32) {
        let pressure = pressure.clamp(0.0, 1.0);
        match self.last {
            None => {
                self.dab(layer, p, self.radius_at(pressure), self.alpha_at(pressure));
                self.last = Some((p, pressure));
                self.carry = 0.0;
            }
            Some((q, qp)) => {
                let d = p - q;
                let dist = d.length();
                if dist < 1e-4 {
                    return;
                }
                // `t` is the position of the previous dab relative to q along the segment.
                let mut t = -self.carry;
                loop {
                    let pr = qp + (pressure - qp) * (t.max(0.0) / dist).min(1.0);
                    let step = (self.radius_at(pr) * 0.12).max(0.35);
                    if t + step > dist {
                        self.carry = dist - t;
                        break;
                    }
                    t += step;
                    let c = q + d * (t / dist);
                    let pr = qp + (pressure - qp) * (t / dist);
                    self.dab(layer, c, self.radius_at(pr), self.alpha_at(pr));
                }
                self.last = Some((p, pressure));
            }
        }
    }

    fn dab(&mut self, layer: &mut RasterLayer, c: Pos2, r: f32, alpha: f32) {
        let x0 = (c.x - r - 1.0).floor() as i32;
        let y0 = (c.y - r - 1.0).floor() as i32;
        let x1 = (c.x + r + 1.0).ceil() as i32;
        let y1 = (c.y + r + 1.0).ceil() as i32;
        let inner = r * self.p.hardness.clamp(0.0, 1.0);
        let soft = (r - inner).max(1e-3);
        let op = self.p.opacity.clamp(0.0, 1.0);
        let col = self.p.color.to_srgba_unmultiplied();
        let cr = col[0] as f32 / 255.0;
        let cg = col[1] as f32 / 255.0;
        let cb = col[2] as f32 / 255.0;

        for ty in y0.div_euclid(TILE)..=y1.div_euclid(TILE) {
            for tx in x0.div_euclid(TILE)..=x1.div_euclid(TILE) {
                let key = (tx, ty);
                let ox = tx * TILE;
                let oy = ty * TILE;
                let sx0 = x0.max(ox);
                let sx1 = x1.min(ox + TILE - 1);
                let sy0 = y0.max(oy);
                let sy1 = y1.min(oy + TILE - 1);
                if sx0 > sx1 || sy0 > sy1 {
                    continue;
                }
                if self.p.erase && !layer.tiles.contains_key(&key) && !self.orig.contains_key(&key) {
                    continue;
                }
                let orig = self.orig.entry(key).or_insert_with(|| layer.tiles.get(&key).cloned()).clone();
                let mask = self.mask.entry(key).or_insert_with(|| vec![0.0; TILE_US * TILE_US]);
                let tile = Arc::make_mut(layer.tiles.entry(key).or_insert_with(|| Arc::new(Tile::empty())));
                for y in sy0..=sy1 {
                    let fy = y as f32 + 0.5 - c.y;
                    for x in sx0..=sx1 {
                        let fx = x as f32 + 0.5 - c.x;
                        let d = (fx * fx + fy * fy).sqrt();
                        if d > r + 0.5 {
                            continue;
                        }
                        let mut a = if d <= inner {
                            1.0
                        } else {
                            let t = ((d - inner) / soft).clamp(0.0, 1.0);
                            1.0 - t * t * (3.0 - 2.0 * t)
                        };
                        a = a.min((r + 0.5 - d).clamp(0.0, 1.0)) * alpha;
                        let li = ((y - oy) * TILE + (x - ox)) as usize;
                        if a <= mask[li] {
                            continue;
                        }
                        mask[li] = a;
                        let pi = li * 4;
                        let base = match &orig {
                            Some(t) => [t.px[pi], t.px[pi + 1], t.px[pi + 2], t.px[pi + 3]],
                            None => [0; 4],
                        };
                        let sa = a * op;
                        if self.p.erase {
                            for k in 0..4 {
                                tile.px[pi + k] = (base[k] as f32 * (1.0 - sa)).round() as u8;
                            }
                        } else {
                            let src = [cr * sa, cg * sa, cb * sa, sa];
                            for k in 0..4 {
                                let v = src[k] * 255.0 + base[k] as f32 * (1.0 - sa);
                                tile.px[pi + k] = v.round().clamp(0.0, 255.0) as u8;
                            }
                        }
                    }
                }
                tile.rev = next_rev();
            }
        }
    }

    /// Drops tiles that ended up fully transparent.
    pub fn finish(self, layer: &mut RasterLayer) {
        for key in self.orig.keys() {
            if layer.tiles.get(key).is_some_and(|t| t.is_empty()) {
                layer.tiles.remove(key);
            }
        }
    }

    /// Restores the layer to the pre-stroke state.
    pub fn cancel(self, layer: &mut RasterLayer) {
        for (key, orig) in self.orig {
            match orig {
                Some(t) => {
                    layer.tiles.insert(key, t);
                }
                None => {
                    layer.tiles.remove(&key);
                }
            }
        }
    }
}
