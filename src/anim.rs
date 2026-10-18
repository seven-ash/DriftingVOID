//! Timeline and keyframes (docs/VIDEO.md, V2).
//!
//! A key is an absolute snapshot of what can move: the layer's centre, an extent
//! (photo scale, text size, shape size), rotation, opacity and, in a 3D space, the
//! pose. Moving the playhead writes the sampled state back into the layers (as in
//! Blender, changes that are not keyed are replaced when the time changes). Layers
//! without keys are never touched.

use egui::{Pos2, Rect, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::doc::{Document, Layer, LayerKind};
use crate::persp::V3;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Ease {
    Linear,
    /// Slow in and out (smoothstep).
    #[default]
    Smooth,
    /// Keep this key's value until the next key.
    Hold,
}

impl Ease {
    pub const ALL: [Ease; 3] = [Ease::Smooth, Ease::Linear, Ease::Hold];

    pub fn label(self) -> &'static str {
        match self {
            Ease::Linear => "직선",
            Ease::Smooth => "부드럽게",
            Ease::Hold => "멈춤",
        }
    }

    fn shape(self, u: f64) -> f64 {
        match self {
            Ease::Linear => u,
            Ease::Smooth => u * u * (3.0 - 2.0 * u),
            Ease::Hold => 0.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct PlaceKey {
    pub pos: V3,
    pub rot: V3,
    pub scale: f64,
    pub thick: f64,
}

/// What a key holds (bits); each value interpolates only between keys holding it.
pub mod ch {
    pub const POS: u16 = 1;
    pub const SIZE: u16 = 2;
    pub const ROT: u16 = 4;
    pub const OPA: u16 = 8;
    pub const P_POS: u16 = 16;
    pub const P_ROT: u16 = 32;
    pub const P_SCALE: u16 = 64;
    pub const P_THICK: u16 = 128;
    pub const FOLLOW: u16 = 256;
    pub const ALL: u16 = 511;
}

/// The values that differ between two states (for auto-key).
pub fn diff_ch(a: &Key, b: &Key) -> u16 {
    let ne = |x: f32, y: f32| (x - y).abs() > 1e-4;
    let mut c = 0;
    if ne(a.center[0], b.center[0]) || ne(a.center[1], b.center[1]) {
        c |= ch::POS;
    }
    if ne(a.ext[0], b.ext[0]) || ne(a.ext[1], b.ext[1]) {
        c |= ch::SIZE;
    }
    if ne(a.rot, b.rot) {
        c |= ch::ROT;
    }
    if ne(a.opacity, b.opacity) {
        c |= ch::OPA;
    }
    if a.follow != b.follow {
        c |= ch::FOLLOW;
    }
    if let (Some(p), Some(q)) = (a.place, b.place) {
        if p.pos != q.pos {
            c |= ch::P_POS;
        }
        if p.rot != q.rot {
            c |= ch::P_ROT;
        }
        if p.scale != q.scale {
            c |= ch::P_SCALE;
        }
        if p.thick != q.thick {
            c |= ch::P_THICK;
        }
    } else if a.place.is_some() != b.place.is_some() {
        c |= ch::P_POS | ch::P_ROT | ch::P_SCALE | ch::P_THICK;
    }
    c
}

fn all_ch() -> u16 {
    ch::ALL
}

/// Copies the values named by `c` from `src` into `dst`.
fn copy_ch(dst: &mut Key, src: &Key, c: u16) {
    if c & ch::POS != 0 {
        dst.center = src.center;
    }
    if c & ch::SIZE != 0 {
        dst.ext = src.ext;
    }
    if c & ch::ROT != 0 {
        dst.rot = src.rot;
    }
    if c & ch::OPA != 0 {
        dst.opacity = src.opacity;
    }
    if c & ch::FOLLOW != 0 {
        dst.follow = src.follow;
    }
    if let Some(sp) = src.place {
        let dp = dst.place.get_or_insert(sp);
        if c & ch::P_POS != 0 {
            dp.pos = sp.pos;
        }
        if c & ch::P_ROT != 0 {
            dp.rot = sp.rot;
        }
        if c & ch::P_SCALE != 0 {
            dp.scale = sp.scale;
        }
        if c & ch::P_THICK != 0 {
            dp.thick = sp.thick;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Key {
    pub t: f64,
    pub center: [f32; 2],
    pub ext: [f32; 2],
    pub rot: f32,
    pub opacity: f32,
    #[serde(default)]
    pub place: Option<PlaceKey>,
    /// Easing towards the next key.
    #[serde(default)]
    pub ease: Ease,
    /// Cameras: the follow constraint at this key (None: not keyed, Some(None): off).
    /// Offsets blend between keys on the same layer; switching holds.
    #[serde(default)]
    pub follow: Option<Option<Follow>>,
    /// Which values this key holds (`ch`); older files hold everything.
    #[serde(default = "all_ch")]
    pub ch: u16,
}

fn lerp(a: f32, b: f32, u: f64) -> f32 {
    a + (b - a) * u as f32
}

fn lerp3(a: V3, b: V3, u: f64) -> V3 {
    [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u]
}

impl Key {
    fn mix(&self, o: &Key, u: f64, t: f64) -> Key {
        Key {
            t,
            center: [lerp(self.center[0], o.center[0], u), lerp(self.center[1], o.center[1], u)],
            ext: [lerp(self.ext[0], o.ext[0], u), lerp(self.ext[1], o.ext[1], u)],
            rot: lerp(self.rot, o.rot, u),
            opacity: lerp(self.opacity, o.opacity, u),
            place: match (self.place, o.place) {
                (Some(a), Some(b)) => Some(PlaceKey {
                    pos: lerp3(a.pos, b.pos, u),
                    rot: lerp3(a.rot, b.rot, u),
                    scale: a.scale + (b.scale - a.scale) * u,
                    thick: a.thick + (b.thick - a.thick) * u,
                }),
                (a, _) => a,
            },
            ease: self.ease,
            follow: match (self.follow, o.follow) {
                (Some(Some(a)), Some(Some(b))) if a.layer == b.layer => Some(Some(Follow {
                    offset: [lerp(a.offset[0], b.offset[0], u), lerp(a.offset[1], b.offset[1], u)],
                    rot_offset: lerp(a.rot_offset, b.rot_offset, u),
                    ..a
                })),
                (a, _) => a,
            },
            ch: self.ch,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Anim {
    /// Sorted by time.
    #[serde(default)]
    pub keys: Vec<Key>,
    /// Shown only from `span[0]` to `span[1]` (seconds); None: always.
    #[serde(default)]
    pub span: Option<[f64; 2]>,
    /// Video layers: the video time at the start of the span.
    #[serde(default)]
    pub video_start: f64,
}

impl Anim {
    pub fn alive(&self, t: f64) -> bool {
        self.span.is_none_or(|[a, b]| t >= a - 1e-9 && t < b - 1e-9)
    }

    pub fn start(&self) -> f64 {
        self.span.map_or(0.0, |s| s[0])
    }

    /// Adds a key, replacing one at (nearly) the same time.
    /// A key at an existing key's time merges into it (only the values it holds).
    pub fn set_key(&mut self, k: Key) {
        if let Some(old) = self.keys.iter_mut().find(|o| (o.t - k.t).abs() < 1e-6) {
            copy_ch(old, &k, k.ch);
            old.ch |= k.ch;
            return;
        }
        let at = self.keys.partition_point(|o| o.t < k.t);
        self.keys.insert(at, k);
    }

    /// Removes the key nearest to `t` within `tol`; true if one went.
    pub fn remove_near(&mut self, t: f64, tol: f64) -> bool {
        let best = self.keys.iter().enumerate().min_by(|a, b| (a.1.t - t).abs().partial_cmp(&(b.1.t - t).abs()).unwrap()).map(|(i, k)| (i, (k.t - t).abs()));
        match best {
            Some((i, d)) if d <= tol => {
                self.keys.remove(i);
                true
            }
            _ => false,
        }
    }

    pub fn sort(&mut self) {
        self.keys.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// State at `t` (held before the first and after the last key), value by
    /// value: each one from the keys that hold it, `base` for values never keyed.
    pub fn sample_on(&self, t: f64, base: &Key) -> Option<Key> {
        if self.keys.is_empty() {
            return None;
        }
        let mut out = Key { t, ..*base };
        for bit in 0..9 {
            let c = 1u16 << bit;
            let ks: Vec<&Key> = self.keys.iter().filter(|k| k.ch & c != 0).collect();
            let (Some(first), Some(last)) = (ks.first(), ks.last()) else { continue };
            let v = if t <= first.t {
                **first
            } else if t >= last.t {
                **last
            } else {
                let i = ks.partition_point(|k| k.t <= t) - 1;
                let (a, b) = (ks[i], ks[i + 1]);
                let u = ((t - a.t) / (b.t - a.t).max(1e-9)).clamp(0.0, 1.0);
                a.mix(b, a.ease.shape(u), t)
            };
            copy_ch(&mut out, &v, c);
        }
        Some(out)
    }

    #[cfg(test)]
    pub fn sample(&self, t: f64) -> Option<Key> {
        let first = *self.keys.first()?;
        self.sample_on(t, &first)
    }
}

/// Playback length, frame rate and the playhead.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Timeline {
    pub on: bool,
    pub fps: f64,
    pub duration: f64,
    pub time: f64,
    /// Key every edit of a selected layer at the playhead.
    #[serde(default)]
    pub autokey: bool,
}

impl Default for Timeline {
    fn default() -> Self {
        Self { on: false, fps: 30.0, duration: 10.0, time: 0.0, autokey: false }
    }
}

impl Timeline {
    pub fn frame_len(&self) -> f64 {
        1.0 / self.fps.max(1.0)
    }

    /// Nearest frame time.
    pub fn snap(&self, t: f64) -> f64 {
        ((t * self.fps).round() / self.fps).clamp(0.0, self.duration)
    }
}

fn unit() -> f32 {
    1.0
}

/// A sound file on the timeline: source from `offset` for `len` seconds, starting
/// at timeline time `start`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct AudioClip {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub start: f64,
    #[serde(default)]
    pub offset: f64,
    /// 0 until the file is decoded the first time.
    pub len: f64,
    #[serde(default = "unit")]
    pub gain: f32,
    #[serde(default)]
    pub mute: bool,
}

impl AudioClip {
    pub fn end(&self) -> f64 {
        self.start + self.len
    }
}

/// A camera on the canvas: a region that can move over time and is shown in its
/// span; image and video export can use it as their region.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct CanvasCam {
    pub id: u64,
    pub name: String,
    pub rect: Rect,
    #[serde(default)]
    pub anim: Anim,
    /// Radians about the centre (clockwise on screen).
    #[serde(default)]
    pub rot: f32,
    /// Follows a layer (constraint), applied after its keys.
    #[serde(default)]
    pub follow: Option<Follow>,
}

/// Camera constraint: keep the centre on a layer (plus `offset`) and/or turn with it
/// (plus `rot_offset`).
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Follow {
    pub layer: u64,
    pub pos: bool,
    pub rot: bool,
    pub offset: [f32; 2],
    pub rot_offset: f32,
}

impl CanvasCam {
    pub fn key(&self, t: f64) -> Key {
        let c = self.rect.center();
        Key { t, center: [c.x, c.y], ext: [self.rect.width(), self.rect.height()], rot: self.rot, opacity: 1.0, place: None, ease: Ease::default(), follow: Some(self.follow), ch: ch::ALL }
    }

    pub fn apply(&mut self, k: &Key) {
        self.rect = Rect::from_center_size(pos2(k.center[0], k.center[1]), vec2(k.ext[0].max(1.0), k.ext[1].max(1.0)));
        self.rot = k.rot;
        if let Some(f) = k.follow {
            self.follow = f;
        }
    }

    /// Corners (TL, TR, BR, BL) in world space, rotation included.
    pub fn corners(&self) -> [Pos2; 4] {
        let r = self.rect;
        let c = r.center();
        [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|p| crate::doc::rot_about(p, c, self.rot))
    }

    /// Is a world point inside the (rotated) frame?
    pub fn contains(&self, p: Pos2) -> bool {
        self.rect.contains(crate::doc::rot_about(p, self.rect.center(), -self.rot))
    }
}

/// Where a layer is and how it is turned, as seen on the canvas (for snapping and
/// constraints).
pub fn target_pose(doc: &Document, l: &Layer) -> Option<(Pos2, f32)> {
    if l.place.is_some() {
        let q = l.shown_quad(&crate::persp::cameras(&doc.spaces))?;
        let c = pos2((q[0].x + q[1].x + q[2].x + q[3].x) * 0.25, (q[0].y + q[1].y + q[2].y + q[3].y) * 0.25);
        return Some((c, (q[1] - q[0]).angle()));
    }
    Some(match &l.kind {
        LayerKind::Image(im) => (im.center, im.rotation),
        LayerKind::Text(tx) => (Rect::from_min_size(tx.pos, crate::text::layout(tx).size).center(), tx.rotation),
        LayerKind::Shape(s) => (s.a + (s.b - s.a) * 0.5, s.rotation),
        _ => (l.bounds()?.center(), 0.0),
    })
}

/// Cameras that follow a layer take its centre and/or rotation.
pub fn apply_constraints(doc: &mut Document) {
    let poses: Vec<(usize, Follow, Option<(Pos2, f32)>)> = doc
        .cameras
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.follow.map(|f| (i, f, doc.get(f.layer).and_then(|l| target_pose(doc, l)))))
        .collect();
    for (i, f, pose) in poses {
        let Some((p, a)) = pose else { continue };
        let c = &mut doc.cameras[i];
        if f.pos {
            c.rect = Rect::from_center_size(p + vec2(f.offset[0], f.offset[1]), c.rect.size());
        }
        if f.rot {
            c.rot = a + f.rot_offset;
        }
    }
}

/// The camera that films at `t`: the first one in the list whose span covers it.
pub fn active_camera(doc: &Document, t: f64) -> Option<&CanvasCam> {
    doc.cameras.iter().find(|c| c.anim.alive(t))
}

/// The animatable state of a layer now.
pub fn key_of(l: &Layer, t: f64) -> Key {
    let (center, ext, rot): (Pos2, Vec2, f32) = match &l.kind {
        LayerKind::Image(im) => (im.center, vec2(im.scale, 0.0), im.rotation),
        LayerKind::Text(tx) => {
            let lay = crate::text::layout(tx);
            (Rect::from_min_size(tx.pos, lay.size).center(), vec2(tx.size, 0.0), tx.rotation)
        }
        LayerKind::Shape(s) => (s.a + (s.b - s.a) * 0.5, s.b - s.a, s.rotation),
        LayerKind::Vector(v) => (v.offset.to_pos2(), Vec2::ZERO, 0.0),
        LayerKind::Raster(r) => (r.offset.to_pos2(), Vec2::ZERO, 0.0),
    };
    Key {
        t,
        center: [center.x, center.y],
        ext: [ext.x, ext.y],
        rot,
        opacity: l.opacity,
        place: l.place.as_ref().map(|p| PlaceKey { pos: p.pos, rot: p.rot, scale: p.scale, thick: p.thick }),
        ease: Ease::default(),
        follow: None,
        ch: ch::ALL,
    }
}

/// Writes a sampled state into a layer.
pub fn apply_key(l: &mut Layer, k: &Key) {
    let c = pos2(k.center[0], k.center[1]);
    match &mut l.kind {
        LayerKind::Image(im) => {
            im.center = c;
            im.scale = k.ext[0].max(1e-4);
            im.rotation = k.rot;
        }
        LayerKind::Text(tx) => {
            tx.size = k.ext[0].max(1.0);
            tx.rotation = k.rot;
            let size = crate::text::layout(tx).size;
            tx.pos = c - size * 0.5;
        }
        LayerKind::Shape(s) => {
            let e = vec2(k.ext[0], k.ext[1]) * 0.5;
            s.a = c - e;
            s.b = c + e;
            s.rotation = k.rot;
        }
        LayerKind::Vector(v) => v.offset = c.to_vec2(),
        LayerKind::Raster(r) => r.offset = c.to_vec2().round(),
    }
    l.opacity = k.opacity.clamp(0.0, 1.0);
    if let (Some(p), Some(pk)) = (l.place.as_mut(), k.place) {
        p.pos = pk.pos;
        p.rot = pk.rot;
        p.scale = pk.scale;
        p.thick = pk.thick;
    }
}

/// Writes every keyed layer's and camera's state at the playhead.
pub fn evaluate(doc: &mut Document) {
    let t = doc.timeline.time;
    for l in &mut doc.layers {
        let base = key_of(l, t);
        if let Some(k) = l.anim.as_ref().and_then(|a| a.sample_on(t, &base)) {
            apply_key(l, &k);
        }
    }
    for c in &mut doc.cameras {
        let base = c.key(t);
        if let Some(k) = c.anim.sample_on(t, &base) {
            c.apply(&k);
        }
    }
    apply_constraints(doc);
}

/// A layer as it shows at the playhead when it has recorded strokes (pen strokes
/// partly drawn, brush strokes replayed). None: draw it as it is.
pub fn layer_at_time(doc: &Document, l: &Layer) -> Option<Layer> {
    if !doc.timeline.on {
        return None;
    }
    let t = doc.timeline.time;
    match &l.kind {
        LayerKind::Vector(v) if v.strokes.iter().any(|s| s.rec.is_some()) => {
            let mut out = l.clone();
            if let LayerKind::Vector(ov) = &mut out.kind {
                ov.strokes = v
                    .strokes
                    .iter()
                    .filter_map(|s| match s.at_time(t) {
                        None => None,
                        Some(None) => Some(s.clone()),
                        Some(Some(part)) => Some(std::sync::Arc::new(part)),
                    })
                    .collect();
            }
            Some(out)
        }
        LayerKind::Raster(r) => {
            let rl = r.replayed(t)?;
            let mut out = l.clone();
            out.kind = LayerKind::Raster(rl);
            Some(out)
        }
        _ => None,
    }
}

/// Video time shown by a video layer at timeline time `t`.
pub fn video_time(anim: Option<&Anim>, speed: f64, t: f64) -> f64 {
    match anim {
        Some(a) => a.video_start + (t - a.start()) * speed,
        None => t * speed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{ShapeKind, ShapeLayer};
    use egui::Color32;

    fn k(t: f64, x: f32) -> Key {
        Key { t, center: [x, 0.0], ext: [1.0, 0.0], rot: 0.0, opacity: 1.0, place: None, ease: Ease::Linear, follow: None, ch: ch::ALL }
    }

    #[test]
    fn sampling_and_easing() {
        let mut a = Anim::default();
        a.set_key(k(2.0, 100.0));
        a.set_key(k(0.0, 0.0));
        assert_eq!(a.keys[0].t, 0.0);
        assert_eq!(a.sample(1.0).unwrap().center[0], 50.0);
        assert_eq!(a.sample(-1.0).unwrap().center[0], 0.0);
        assert_eq!(a.sample(5.0).unwrap().center[0], 100.0);
        a.keys[0].ease = Ease::Hold;
        assert_eq!(a.sample(1.9).unwrap().center[0], 0.0);
        a.keys[0].ease = Ease::Smooth;
        let s = a.sample(0.5).unwrap().center[0];
        assert!(s < 25.0 && s > 0.0, "{s}");
        // replacing keeps the ease
        a.set_key(k(0.0, 10.0));
        assert_eq!((a.keys.len(), a.keys[0].ease, a.keys[0].center[0]), (2, Ease::Smooth, 10.0));
        assert!(a.remove_near(2.02, 0.05));
        assert_eq!(a.keys.len(), 1);
    }

    #[test]
    fn spans() {
        let a = Anim { span: Some([1.0, 3.0]), ..Default::default() };
        assert!(!a.alive(0.5) && a.alive(1.0) && a.alive(2.9) && !a.alive(3.0));
        assert!((video_time(Some(&Anim { span: Some([2.0, 9.0]), video_start: 5.0, ..Default::default() }), 2.0, 3.0) - 7.0).abs() < 1e-9);
    }

    #[test]
    fn shape_round_trip() {
        let mut doc = Document::default();
        let id = doc.insert(
            "s".into(),
            LayerKind::Shape(ShapeLayer { kind: ShapeKind::Rect, a: pos2(0.0, 0.0), b: pos2(100.0, 50.0), color: Color32::WHITE, width: 2.0, fill: None, rotation: 0.3 }),
            None,
        );
        let l = doc.get_mut(id).unwrap();
        let k0 = key_of(l, 0.0);
        let mut k1 = k0;
        k1.t = 1.0;
        k1.center = [200.0, 25.0];
        k1.opacity = 0.0;
        l.anim = Some(Anim { keys: vec![k0, k1], ..Default::default() });
        doc.timeline.time = 0.5;
        evaluate(&mut doc);
        let l = doc.get(id).unwrap();
        let LayerKind::Shape(s) = &l.kind else { panic!() };
        assert!((s.a.x - 75.0).abs() < 1e-3, "{:?}", s.a);
        assert_eq!(s.b - s.a, vec2(100.0, 50.0));
        assert!((l.opacity - 0.5).abs() < 1e-6);
        assert_eq!(key_of(l, 0.5).center, [125.0, 25.0]);
    }

    #[test]
    fn recorded_pen_stroke_draws_in() {
        use crate::doc::VectorLayer;
        use crate::vector::VStroke;
        use std::sync::Arc;
        let pts: Vec<[f32; 3]> = (0..11).map(|i| [i as f32 * 10.0, 0.0, 2.0]).collect();
        let times: Vec<f32> = (0..11).map(|i| i as f32 * 0.1).collect();
        let st = VStroke::timed(pts, times, Color32::WHITE, 2.0);
        let mut doc = Document::default();
        let id = doc.insert("p".into(), LayerKind::Vector(VectorLayer { offset: Vec2::ZERO, strokes: vec![Arc::new(st)] }), None);
        doc.timeline.on = true;
        let count = |doc: &Document| match layer_at_time(doc, doc.get(id).unwrap()).map(|l| l.kind) {
            Some(LayerKind::Vector(v)) => v.strokes.first().map_or(0, |s| s.pts.len()),
            _ => 99,
        };
        doc.timeline.time = 1.0;
        assert_eq!(count(&doc), 0, "not started");
        doc.timeline.time = 2.45;
        assert_eq!(count(&doc), 5, "half drawn");
        doc.timeline.time = 9.0;
        assert_eq!(count(&doc), 11, "complete");
    }

    #[test]
    fn recorded_brush_replays() {
        use crate::raster::{BrushParams, BrushRec, RasterLayer, RasterStroke};
        use std::sync::Arc;
        let params = BrushParams { color: Color32::RED, radius: 6.0, opacity: 1.0, hardness: 1.0, pressure_size: false, pressure_opacity: false, erase: false };
        let mut r = RasterLayer { rec_base: Some(Default::default()), ..Default::default() };
        // two strokes drawn at t = 0 (left) and t = 5 (right), each 1 s long
        for (start, x0) in [(0.0, 0.0f32), (5.0, 300.0)] {
            let pts: Vec<[f32; 3]> = (0..=10).map(|i| [x0 + i as f32 * 10.0, 50.0, 1.0]).collect();
            let times: Vec<f32> = (0..=10).map(|i| i as f32 * 0.1).collect();
            let mut st = RasterStroke::new(params);
            for p in &pts {
                st.add_point(&mut r, egui::pos2(p[0], p[1]), p[2]);
            }
            st.finish(&mut r);
            r.recs.push(Arc::new(BrushRec { start, params, pts, times }));
        }
        assert!(r.replayed(7.0).is_none(), "after the end: the final pixels");
        let mid = r.replayed(0.5).unwrap();
        assert!(mid.pixel(20, 50)[3] > 200 && mid.pixel(90, 50)[3] == 0, "first stroke half drawn");
        assert_eq!(mid.pixel(350, 50)[3], 0, "second not yet");
        let later = r.replayed(5.55).unwrap();
        assert!(later.pixel(90, 50)[3] > 200 && later.pixel(340, 50)[3] > 200 && later.pixel(395, 50)[3] == 0);
    }

    #[test]
    fn camera_follows_a_layer() {
        let mut doc = Document::default();
        let id = doc.insert(
            "s".into(),
            LayerKind::Shape(ShapeLayer { kind: ShapeKind::Rect, a: pos2(100.0, 100.0), b: pos2(200.0, 150.0), color: Color32::WHITE, width: 1.0, fill: None, rotation: 0.5 }),
            None,
        );
        doc.cameras.push(CanvasCam {
            id: 9,
            name: "c".into(),
            rect: Rect::from_center_size(pos2(0.0, 0.0), vec2(160.0, 90.0)),
            anim: Anim::default(),
            rot: 0.0,
            follow: Some(Follow { layer: id, pos: true, rot: true, offset: [10.0, 0.0], rot_offset: 0.1 }),
        });
        apply_constraints(&mut doc);
        let c = &doc.cameras[0];
        assert_eq!(c.rect.center(), pos2(160.0, 125.0));
        assert_eq!(c.rect.size(), vec2(160.0, 90.0));
        assert!((c.rot - 0.6).abs() < 1e-6);
        assert!(c.contains(pos2(160.0, 125.0)));
    }

    #[test]
    fn camera_follow_and_orientation_are_keyed() {
        let mut cam = CanvasCam { id: 1, name: "c".into(), rect: Rect::from_center_size(pos2(0.0, 0.0), vec2(160.0, 90.0)), anim: Anim::default(), rot: 0.0, follow: None };
        let k0 = cam.key(0.0);
        cam.rect = Rect::from_center_size(pos2(0.0, 0.0), vec2(90.0, 160.0));
        cam.follow = Some(Follow { layer: 5, pos: true, rot: false, offset: [0.0, 0.0], rot_offset: 0.0 });
        let mut k1 = cam.key(2.0);
        k1.ease = Ease::Hold;
        let mut k0 = k0;
        k0.ease = Ease::Hold;
        cam.anim.keys = vec![k0, k1];
        cam.apply(&cam.anim.sample(1.0).unwrap());
        assert_eq!((cam.rect.width(), cam.follow), (160.0, None));
        cam.apply(&cam.anim.sample(2.5).unwrap());
        assert_eq!(cam.rect.width(), 90.0);
        assert_eq!(cam.follow.map(|f| f.layer), Some(5));
    }

    #[test]
    fn values_key_on_their_own() {
        // opacity keyed at 0 and 2, position only at 1: position holds, opacity blends
        let mut a = Anim::default();
        let mut k0 = k(0.0, 0.0);
        k0.opacity = 0.0;
        k0.ch = ch::OPA;
        let mut k2 = k(2.0, 0.0);
        k2.opacity = 1.0;
        k2.ch = ch::OPA;
        let mut k1 = k(1.0, 300.0);
        k1.ch = ch::POS;
        a.set_key(k0);
        a.set_key(k2);
        a.set_key(k1);
        let mut base = k(0.0, -5.0);
        base.opacity = 0.5;
        let v = a.sample_on(1.0, &base).unwrap();
        assert_eq!(v.center[0], 300.0);
        assert!((v.opacity - 0.5).abs() < 1e-6);
        let v = a.sample_on(0.0, &base).unwrap();
        assert_eq!(v.center[0], 300.0);
        assert_eq!(v.opacity, 0.0);
        // a key at the same time merges, keeping the other values
        let mut k1b = k(1.0, 0.0);
        k1b.opacity = 0.25;
        k1b.ch = ch::OPA;
        a.set_key(k1b);
        let m = a.keys.iter().find(|k| k.t == 1.0).unwrap();
        assert_eq!(m.ch, ch::POS | ch::OPA);
        assert_eq!(m.center[0], 300.0);
        assert_eq!(m.opacity, 0.25);
    }
}
