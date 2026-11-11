// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.

//! Timeline and keyframes (notes/VIDEO.md, V2).
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
            Ease::Linear => crate::i18n::tr("직선"),
            Ease::Smooth => crate::i18n::tr("부드럽게"),
            Ease::Hold => crate::i18n::tr("멈춤"),
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
    /// Sound volume (video layers, sound clips).
    pub const VOL: u16 = 512;
    /// An FX layer's values, one channel each (`FxLayer::v`).
    pub const FX: [u16; 4] = [1024, 2048, 4096, 8192];
    /// Every value (keys are made of what a layer has: `channels_of`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub const ALL: u16 = 16383;
    /// What a canvas camera has to key.
    pub const CAMERA: u16 = POS | SIZE | ROT | FOLLOW;
    /// Everything a key held before volume existed (older files).
    pub const OLD_ALL: u16 = 511;
    /// Number of value bits.
    pub const COUNT: u16 = 14;
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
    if ne(a.vol, b.vol) {
        c |= ch::VOL;
    }
    for i in 0..4 {
        if ne(a.fx[i], b.fx[i]) {
            c |= ch::FX[i];
        }
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
    ch::OLD_ALL
}

fn unit_vol() -> f32 {
    1.0
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
    if c & ch::VOL != 0 {
        dst.vol = src.vol;
    }
    for i in 0..4 {
        if c & ch::FX[i] != 0 {
            dst.fx[i] = src.fx[i];
        }
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
    /// Which values this key holds (`ch`); older files hold everything they knew.
    #[serde(default = "all_ch")]
    pub ch: u16,
    /// Sound volume (1 = as recorded).
    #[serde(default = "unit_vol")]
    pub vol: f32,
    /// An FX layer's values.
    #[serde(default)]
    pub fx: [f32; 4],
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
            vol: lerp(self.vol, o.vol, u),
            fx: std::array::from_fn(|i| lerp(self.fx[i], o.fx[i], u)),
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
    /// Keys `k`'s values, one key per value (Ash, 10-29: each family of keys
    /// stands on its own; a key holds one channel): into that channel's key
    /// at the same time, else a new one.
    pub fn set_key(&mut self, k: Key) {
        for bit in 0..ch::COUNT {
            let c = 1u16 << bit;
            if k.ch & c == 0 {
                continue;
            }
            if let Some(old) = self.keys.iter_mut().find(|o| o.ch == c && (o.t - k.t).abs() < 1e-6) {
                copy_ch(old, &k, c);
                continue;
            }
            let at = self.keys.partition_point(|o| o.t <= k.t);
            self.keys.insert(at, Key { ch: c, ..k });
        }
    }

    /// One key per channel: keys holding several (files before 0.97.0) are
    /// split, each part with the same time, values and easing. True if any was.
    pub fn split_channels(&mut self) -> bool {
        if self.keys.iter().all(|k| k.ch.count_ones() <= 1) {
            return false;
        }
        let mut out = Vec::with_capacity(self.keys.len() * 4);
        for k in &self.keys {
            for bit in 0..ch::COUNT {
                let c = 1u16 << bit;
                if k.ch & c != 0 {
                    out.push(Key { ch: c, ..*k });
                }
            }
        }
        self.keys = out;
        self.sort();
        true
    }

    /// Removes the keys at the time nearest to `t` within `tol` (every
    /// channel's); true if any went.
    pub fn remove_near(&mut self, t: f64, tol: f64) -> bool {
        let best = self.keys.iter().map(|k| k.t).min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()));
        match best {
            Some(kt) if (kt - t).abs() <= tol => {
                self.keys.retain(|k| (k.t - kt).abs() >= 1e-6);
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
        for bit in 0..ch::COUNT {
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
        // (a file may state fps 0: as `frame_len`)
        let fps = self.fps.max(1.0);
        ((t * fps).round() / fps).clamp(0.0, self.duration.max(0.0))
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
    /// Volume keys (VOL only).
    #[serde(default)]
    pub anim: Anim,
}

/// Volume over timeline time from keys (`None` when nothing is keyed): samples
/// every 10 ms from `t0` to `t1`, for the sound mixer.
pub fn vol_envelope(anim: &Anim, t0: f64, t1: f64) -> Option<crate::audio::Envelope> {
    if !anim.keys.iter().any(|k| k.ch & ch::VOL != 0) {
        return None;
    }
    let dt = 0.01;
    let t1 = t1.min(t0 + 3600.0).max(t0);
    let n = ((t1 - t0) / dt).ceil() as usize + 2;
    let base = anim.keys[0];
    let v: Vec<f32> = (0..n).map(|i| anim.sample_on(t0 + i as f64 * dt, &base).map_or(1.0, |k| k.vol)).collect();
    Some(crate::audio::Envelope { t0, dt, v })
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
        Key { t, center: [c.x, c.y], ext: [self.rect.width(), self.rect.height()], rot: self.rot, opacity: 1.0, place: None, ease: Ease::default(), follow: Some(self.follow), ch: ch::CAMERA, vol: 1.0, fx: [0.0; 4] }
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
        LayerKind::Audio(a) => (a.pos, Vec2::ZERO, 0.0),
        LayerKind::Fx(_) => (Pos2::ZERO, Vec2::ZERO, 0.0),
    };
    let fx = match &l.kind {
        LayerKind::Fx(f) => Some(f.v),
        _ => None,
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
        // only video and sound layers have a volume to key
        ch: channels_of(l),
        vol: sound_vol(l).unwrap_or(1.0),
        fx: fx.unwrap_or_default(),
    }
}

/// The values a layer has to key (Ash, 10-29: a key row per value it has):
/// a sound only its volume; an FX layer its opacity and its filter's
/// values; pictures their place, opacity, and size and turn where they have
/// them; a layer in a space its 3D values; a video its volume too.
pub fn channels_of(l: &Layer) -> u16 {
    let mut c = match &l.kind {
        LayerKind::Audio(_) => 0,
        LayerKind::Fx(fx) => ch::OPA | ch::FX.iter().take(fx.filter.params().len()).fold(0, |a, b| a | b),
        LayerKind::Image(_) | LayerKind::Text(_) | LayerKind::Shape(_) => ch::POS | ch::SIZE | ch::ROT | ch::OPA,
        LayerKind::Vector(_) | LayerKind::Raster(_) => ch::POS | ch::OPA,
    };
    if l.place.is_some() {
        c |= ch::P_POS | ch::P_ROT | ch::P_SCALE | ch::P_THICK;
    }
    if sound_vol(l).is_some() {
        c |= ch::VOL;
    }
    c
}

/// The state `l` shows at `t`: its own values, its keyed ones as sampled there.
pub fn posed_key(l: &Layer, t: f64) -> Key {
    let base = key_of(l, t);
    l.anim.as_ref().and_then(|a| a.sample_on(t, &base)).unwrap_or(base)
}

/// Auto-key: the values the user changed at `t`, `before` (the last
/// committed layer) against `now`. Both as they show at `t`, so a value only
/// eased there by its keys never counts as changed (Ash, 10-29: keying the
/// size also keyed the opacity at its eased value, because the committed
/// layer still held the opacity of the time it was committed at).
pub fn autokey_channels(before: &Layer, now: &Layer, t: f64) -> u16 {
    let k = key_of(now, t);
    diff_ch(&posed_key(before, t), &k) & k.ch
}

/// As `autokey_channels`, for a canvas camera.
pub fn autokey_channels_cam(before: &CanvasCam, now: &CanvasCam, t: f64) -> u16 {
    let base = before.key(t);
    let prev = before.anim.sample_on(t, &base).unwrap_or(base);
    let k = now.key(t);
    diff_ch(&prev, &k) & k.ch
}

/// A video or sound layer's volume.
fn sound_vol(l: &Layer) -> Option<f32> {
    match &l.kind {
        LayerKind::Image(im) => im.video.as_ref().map(|v| v.volume),
        LayerKind::Audio(a) => Some(a.volume),
        _ => None,
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
            if let Some(v) = im.video.as_mut() {
                v.volume = k.vol.max(0.0);
            }
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
        LayerKind::Audio(a) => {
            a.pos = c;
            a.volume = k.vol.max(0.0);
        }
        LayerKind::Fx(f) => {
            for (i, p) in f.filter.params().iter().enumerate() {
                f.v[i] = k.fx[i].clamp(p.lo, p.hi);
            }
        }
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

/// `video_time` for a video layer: a looping one (GIFs, or 반복 ticked)
/// starts over past its end instead of holding the last frame.
pub fn clip_time(anim: Option<&Anim>, v: &crate::doc::VideoRef, t: f64) -> f64 {
    let vt = video_time(anim, v.speed, t);
    let d = v.info.duration;
    if v.looping && d > 0.0 && vt >= d { vt.rem_euclid(d) } else { vt }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{ShapeKind, ShapeLayer};

    /// A file that states fps 0 still snaps to a time (before 0.105.1: NaN).
    #[test]
    fn snapping_survives_a_zero_frame_rate() {
        let tl = Timeline { fps: 0.0, ..Default::default() };
        assert_eq!(tl.snap(2.4), 2.0);
        assert_eq!(Timeline::default().snap(1.02), 1.0 + 1.0 / 30.0);
    }
    use egui::Color32;

    #[test]
    fn clip_volume_keys_make_an_envelope() {
        // a sound layer keys its volume like a video layer
        let mut doc = Document::default();
        let id = doc.insert("a".into(), LayerKind::Audio(crate::doc::AudioLayer { path: "a.wav".into(), pos: pos2(0.0, 0.0), len: 4.0, volume: 1.0, mute: false }), None);
        let l = doc.get_mut(id).unwrap();
        let mut anim = Anim::default();
        let mut k0 = key_of(l, 1.0);
        assert!(k0.ch & ch::VOL != 0);
        k0.ease = Ease::Linear;
        anim.set_key(k0);
        if let LayerKind::Audio(a) = &mut l.kind {
            a.volume = 0.0;
        }
        anim.set_key(key_of(l, 3.0));
        let e = vol_envelope(&anim, 1.0, 5.0).unwrap();
        assert!((e.at(1.0) - 1.0).abs() < 1e-3);
        assert!((e.at(2.0) - 0.5).abs() < 1e-2, "{}", e.at(2.0));
        assert!(e.at(4.5).abs() < 1e-3, "holds after the last key");
        // a key from an older file holds everything it knew, not the volume
        let old: Key = serde_json::from_str(r#"{"t":0,"center":[0,0],"ext":[1,0],"rot":0,"opacity":1}"#).unwrap();
        assert_eq!(old.ch & ch::VOL, 0);
        assert_eq!(old.vol, 1.0);
    }

    fn k(t: f64, x: f32) -> Key {
        Key { t, center: [x, 0.0], ext: [1.0, 0.0], rot: 0.0, opacity: 1.0, place: None, ease: Ease::Linear, follow: None, ch: ch::ALL, vol: 1.0, fx: [0.0; 4] }
    }

    /// The position channel's keys (every value has its own key).
    fn pos_keys(a: &mut Anim) -> Vec<&mut Key> {
        a.keys.iter_mut().filter(|k| k.ch == ch::POS).collect()
    }

    /// Auto-key keys only what changed: with opacity keyed 1 → 0 over 2 s and
    /// the layer committed at 0 s, moving to 1 s and changing the size keys
    /// the size alone, not the opacity at its eased 0.5 (Ash, 10-29).
    #[test]
    fn autokey_keys_only_what_changed() {
        use crate::doc::{Document, LayerKind};
        let mut doc = Document::default();
        let shape = crate::doc::ShapeLayer { kind: crate::doc::ShapeKind::Rect, a: Pos2::ZERO, b: pos2(10.0, 10.0), color: egui::Color32::RED, fill: None, width: 1.0, rotation: 0.0 };
        let id = doc.insert("s".into(), LayerKind::Shape(shape), None);
        let mut a = Anim::default();
        for (t, o) in [(0.0, 1.0f32), (2.0, 0.0)] {
            let mut k = key_of(doc.get(id).unwrap(), t);
            k.ch = ch::OPA;
            k.opacity = o;
            k.ease = Ease::Linear;
            a.set_key(k);
        }
        doc.get_mut(id).unwrap().anim = Some(a);
        doc.timeline.on = true;
        evaluate(&mut doc);
        let committed = doc.get(id).unwrap().clone();
        // the playhead moves to 1 s: the opacity eases to 0.5; then the size changes
        doc.timeline.time = 1.0;
        evaluate(&mut doc);
        let l = doc.get_mut(id).unwrap();
        assert!((l.opacity - 0.5).abs() < 1e-4);
        if let LayerKind::Shape(s) = &mut l.kind {
            s.b = pos2(30.0, 20.0);
        }
        let now = doc.get(id).unwrap().clone();
        // what auto-key did before: the opacity counted as changed
        assert_ne!(diff_ch(&key_of(&committed, 1.0), &key_of(&now, 1.0)) & ch::OPA, 0, "the bug's precondition");
        assert_eq!(autokey_channels(&committed, &now, 1.0), ch::POS | ch::SIZE, "the size (and its centre), not the opacity");
    }

    /// Keys are made only of the values a layer or camera has (one timeline
    /// row each): no 3D rows on a flat photo, no opacity on a camera.
    #[test]
    fn keys_hold_what_a_layer_has() {
        use crate::doc::{Document, Filter, FxLayer, LayerKind};
        let mut doc = Document::default();
        let shape = crate::doc::ShapeLayer { kind: crate::doc::ShapeKind::Rect, a: Pos2::ZERO, b: pos2(10.0, 10.0), color: egui::Color32::RED, fill: None, width: 1.0, rotation: 0.0 };
        let s = doc.insert("s".into(), LayerKind::Shape(shape), None);
        let f = doc.insert("f".into(), LayerKind::Fx(FxLayer::new(Filter::Unsharp)), None);
        let pen = doc.insert("p".into(), LayerKind::Vector(crate::doc::VectorLayer { offset: Vec2::ZERO, strokes: Vec::new() }), None);
        let c = |id| key_of(doc.get(id).unwrap(), 0.0).ch;
        assert_eq!(c(s), ch::POS | ch::SIZE | ch::ROT | ch::OPA);
        assert_eq!(c(f), ch::OPA | ch::FX[0] | ch::FX[1] | ch::FX[2], "unsharp has three values");
        assert_eq!(c(pen), ch::POS | ch::OPA);
        let mut a = Anim::default();
        a.set_key(key_of(doc.get(s).unwrap(), 1.0));
        assert_eq!(a.keys.len(), 4, "one key per value");
        let cam = CanvasCam { id: 1, name: "c".into(), rect: egui::Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)), anim: Anim::default(), rot: 0.0, follow: None };
        assert_eq!(cam.key(0.0).ch, ch::CAMERA);
    }

    #[test]
    fn sampling_and_easing() {
        let mut a = Anim::default();
        a.set_key(k(2.0, 100.0));
        a.set_key(k(0.0, 0.0));
        assert_eq!(a.keys[0].t, 0.0);
        // one key per value: every channel keyed holds a key of its own
        assert!(a.keys.iter().all(|k| k.ch.count_ones() == 1));
        assert_eq!(a.sample(1.0).unwrap().center[0], 50.0);
        assert_eq!(a.sample(-1.0).unwrap().center[0], 0.0);
        assert_eq!(a.sample(5.0).unwrap().center[0], 100.0);
        pos_keys(&mut a)[0].ease = Ease::Hold;
        assert_eq!(a.sample(1.9).unwrap().center[0], 0.0);
        pos_keys(&mut a)[0].ease = Ease::Smooth;
        let s = a.sample(0.5).unwrap().center[0];
        assert!(s < 25.0 && s > 0.0, "{s}");
        // replacing keeps the ease
        let n = a.keys.len();
        a.set_key(k(0.0, 10.0));
        let p = pos_keys(&mut a);
        assert_eq!((p.len(), p[0].ease, p[0].center[0]), (2, Ease::Smooth, 10.0));
        assert_eq!(a.keys.len(), n);
        // Alt+I: every value's key at that time goes
        assert!(a.remove_near(2.02, 0.05));
        assert!(a.keys.iter().all(|k| k.t == 0.0));
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
        // a key of another value at the same time is a key of its own (Ash,
        // 10-29: families of keys apart); the position's stays as it was
        let mut k1b = k(1.0, 0.0);
        k1b.opacity = 0.25;
        k1b.ch = ch::OPA;
        a.set_key(k1b);
        let at1: Vec<&Key> = a.keys.iter().filter(|k| k.t == 1.0).collect();
        assert_eq!(at1.len(), 2);
        assert!(at1.iter().any(|k| k.ch == ch::POS && k.center[0] == 300.0));
        assert!(at1.iter().any(|k| k.ch == ch::OPA && k.opacity == 0.25));
        let v = a.sample_on(1.0, &base).unwrap();
        assert_eq!((v.center[0], v.opacity), (300.0, 0.25));
        // old keys holding several values split into one per value, same sampling
        let mut old = Anim::default();
        old.keys = vec![k(0.0, 0.0), k(2.0, 100.0)];
        let before = old.sample(0.7).unwrap();
        assert!(old.split_channels());
        assert!(old.keys.iter().all(|k| k.ch.count_ones() == 1));
        let after = old.sample(0.7).unwrap();
        assert_eq!((before.center, before.opacity, before.ext), (after.center, after.opacity, after.ext));
    }
}
