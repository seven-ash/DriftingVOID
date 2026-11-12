// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (AGPL-3.0 section 7): see NOTICE.

//! Playback cache (render overhaul phase 4, notes/updates/wgpu-renderer.md).
//!
//! Frames of the export region (a camera or the export frame, at its own pixels,
//! the long side at most 3840), or of the canvas view when there is no region,
//! keyed by frame number, rendered ahead by a worker thread on the GPU export
//! renderer from the last committed document. A region's frames survive
//! panning and zooming. While the timeline plays, a cached frame is shown in
//! place of drawing the document live; frames that aren't cached yet are drawn
//! live as before. Filled ahead of the playhead while playing, and around it
//! after a short idle. RAM: as much as needed, up to 75 % of physical memory,
//! given back when less than 1 GB is free (Ash, 10-20 and 10-21).

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, mpsc};

use egui::{ColorImage, Pos2, Vec2};

use crate::doc::{Document, LayerId, LayerKind};

/// What the cache films: the canvas view.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct View {
    /// Canvas-space centre and size of the view.
    pub center: Pos2,
    pub size: Vec2,
    /// Output pixels per canvas unit (zoom × pixels per point).
    pub scale: f32,
}

/// What the cache films.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Target {
    /// The canvas view (no region): lost on every pan or zoom.
    View(View),
    /// A canvas camera, wherever its keys take it.
    Camera(u64),
    /// The export frame.
    Frame(egui::Rect),
}

/// The long side of a region's frames.
const MAX_SIDE: f32 = 3840.0;

struct Job {
    epoch: u64,
    doc: Arc<Document>,
    target: Target,
    size: (u32, u32),
    fps: f64,
    frames: Vec<u64>,
}

enum Msg {
    Frame { epoch: u64, idx: u64, img: Arc<ColorImage> },
    /// The job's list is done (or the GPU is missing).
    Done { epoch: u64, ok: bool },
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Off,
    Ahead,
    Idle,
}

pub struct PlayCache {
    frames: BTreeMap<u64, Arc<ColorImage>>,
    bytes: usize,
    /// Bumped whenever cached frames stop being valid as a whole (view, fps).
    epoch: u64,
    target: Option<Target>,
    /// Frame size in pixels.
    size: (u32, u32),
    fps: f64,
    duration: f64,
    /// Per layer: its fingerprint, time span and keys (`Print`).
    revs: HashMap<LayerId, Print>,
    global: u64,
    doc_rev: Option<u64>,
    doc: Option<Arc<Document>>,
    tx: Option<mpsc::Sender<Job>>,
    rx: Option<mpsc::Receiver<Msg>>,
    /// The job out at the worker: (epoch, mode, first frame) — None when idle.
    out: Option<(u64, Mode, u64)>,
    tex: Option<egui::TextureHandle>,
    shown: Option<(u64, u64)>,
    /// No GPU (or `DV_NO_CACHE`): never used.
    pub off: bool,
    /// Frames the worker has made (`DV_TIMING`).
    pub made: u64,
    /// When what is filmed last changed (target, frame size, spaces,
    /// cameras, export frame).
    since: std::time::Instant,
}

impl PlayCache {
    pub fn new() -> Self {
        PlayCache {
            frames: BTreeMap::new(),
            bytes: 0,
            epoch: 0,
            target: None,
            size: (0, 0),
            fps: 0.0,
            duration: 0.0,
            revs: HashMap::new(),
            global: 0,
            doc_rev: None,
            doc: None,
            tx: None,
            rx: None,
            out: None,
            tex: None,
            shown: None,
            off: std::env::var_os("DV_NO_CACHE").is_some(),
            made: 0,
            since: std::time::Instant::now(),
        }
    }

    fn clear(&mut self) {
        self.frames.clear();
        self.bytes = 0;
        self.epoch += 1;
        self.shown = None;
    }

    /// Follows the committed document (`rev` changes on every commit, undo and
    /// load) and what is filmed: changed layers drop the frames of their time
    /// span; a new target or size, fps, space or camera drops everything.
    pub fn sync(&mut self, rev: u64, committed: &Document, target: Target) {
        if self.off {
            return;
        }
        let tl = &committed.timeline;
        if self.target != Some(target) || self.fps != tl.fps {
            self.target = Some(target);
            self.fps = tl.fps;
            self.since = std::time::Instant::now();
            self.clear();
            // the size comes with the document pass below
            self.doc_rev = None;
        }
        self.duration = tl.duration;
        if self.doc_rev == Some(rev) {
            return;
        }
        self.doc_rev = Some(rev);
        self.doc = Some(Arc::new(committed.clone()));
        let size = frame_size(committed, target);
        if size != self.size {
            self.size = size;
            self.since = std::time::Instant::now();
            self.clear();
        }
        let (revs, global) = fingerprints(committed);
        if global != self.global {
            self.global = global;
            self.revs = revs;
            self.since = std::time::Instant::now();
            self.clear();
            return;
        }
        // spans of layers that changed, appeared or went away; a layer whose
        // keys alone changed, only between the keys around the change
        let mut spans: Vec<Option<[f64; 2]>> = Vec::new();
        for (id, p) in &revs {
            match self.revs.get(id) {
                Some(p0) if p0.full == p.full => {}
                Some(p0) => match committed.get(*id).and_then(|now| keys_changed(p0, p, now)) {
                    Some(r) => spans.push(Some(r)),
                    None => {
                        spans.push(p0.span);
                        spans.push(p.span);
                    }
                },
                None => spans.push(p.span),
            }
        }
        for (id, p0) in &self.revs {
            if !revs.contains_key(id) {
                spans.push(p0.span);
            }
        }
        self.revs = revs;
        if spans.is_empty() {
            return;
        }
        if spans.iter().any(|s| s.is_none()) {
            self.clear();
            return;
        }
        let fps = self.fps.max(1.0);
        let drop: Vec<u64> = self
            .frames
            .keys()
            .copied()
            .filter(|i| {
                let t = *i as f64 / fps;
                spans.iter().flatten().any(|[a, b]| t >= a - 1.0 / fps && t <= b + 1.0 / fps)
            })
            .collect();
        for i in drop {
            if let Some(f) = self.frames.remove(&i) {
                self.bytes -= frame_bytes(&f);
            }
        }
        // the worker's current list may hold dropped frames: send it again
        self.epoch += 1;
        self.shown = None;
    }

    /// Takes finished frames and keeps the worker busy: ahead of the playhead
    /// while `playing`, around it when `idle`, else resting.
    pub fn pump(&mut self, ctx: &egui::Context, playing: bool, idle: bool, playhead: f64) {
        if self.off || self.target.is_none() || self.doc.is_none() || self.size.0 < 2 || self.size.1 < 2 {
            return;
        }
        if self.tx.is_none() {
            let (jtx, jrx) = mpsc::channel::<Job>();
            let (mtx, mrx) = mpsc::channel::<Msg>();
            let c = ctx.clone();
            std::thread::spawn(move || worker(jrx, mtx, c));
            self.tx = Some(jtx);
            self.rx = Some(mrx);
        }
        let fps = self.fps.max(1.0);
        let last = ((self.duration * fps).ceil() as u64).saturating_sub(1);
        let head = ((playhead * fps).round() as u64).min(last);
        let mut done = false;
        while let Some(m) = self.rx.as_ref().and_then(|r| r.try_recv().ok()) {
            match m {
                Msg::Frame { epoch, idx, img } if epoch == self.epoch => {
                    if !self.frames.contains_key(&idx) {
                        self.bytes += frame_bytes(&img);
                        self.frames.insert(idx, img);
                    }
                    self.made += 1;
                }
                Msg::Done { epoch, ok } => {
                    if !ok {
                        self.off = true;
                    }
                    if self.out.is_some_and(|o| o.0 == epoch) {
                        done = true;
                    }
                }
                _ => {}
            }
        }
        if done {
            self.out = None;
        }
        // memory: frames farthest from the playhead go first (while the system
        // runs low, one per frame, so other programs get it back gradually)
        let (cap, low) = budget();
        while (self.bytes > cap || low) && self.frames.len() > 2 {
            let far = *self.frames.keys().max_by_key(|i| i.abs_diff(head)).unwrap();
            let f = self.frames.remove(&far).unwrap();
            self.bytes -= frame_bytes(&f);
            if low {
                break;
            }
        }
        let mode = if playing {
            Mode::Ahead
        } else if idle {
            Mode::Idle
        } else {
            Mode::Off
        };
        // a new list when the mode or the frames' validity changed, the last list
        // ran out, or the playhead jumped away from where the list started
        let want_new = match self.out {
            None => mode != Mode::Off,
            Some((g, m, first)) => g != self.epoch || m != mode || (mode == Mode::Ahead && (head < first.saturating_sub(1) || head > first + (2.0 * fps) as u64 && !self.frames.contains_key(&(head + 1)))),
        };
        if !want_new {
            return;
        }
        let per = self.frames.values().next().map_or(0, |f| frame_bytes(f)).max(1);
        let room = cap.saturating_sub(self.bytes) / per;
        let mut list = Vec::new();
        if mode != Mode::Off && !low {
            // from just after the playhead to the end, then from the start
            let start = if mode == Mode::Ahead { head + 1 } else { head };
            for i in (start..=last).chain(0..start.min(last + 1)) {
                if list.len() as u64 >= room.max(1) as u64 && !self.frames.is_empty() {
                    break;
                }
                if !self.frames.contains_key(&i) {
                    list.push(i);
                }
            }
        }
        let (Some(tx), Some(doc), Some(target)) = (&self.tx, &self.doc, self.target) else { return };
        let _ = tx.send(Job { epoch: self.epoch, doc: doc.clone(), target, size: self.size, fps, frames: list.clone() });
        self.out = if list.is_empty() { None } else { Some((self.epoch, mode, head)) };
    }

    /// The cached frame at `t` as a texture (uploaded once per shown frame).
    pub fn texture(&mut self, ctx: &egui::Context, t: f64) -> Option<egui::TextureId> {
        if self.off {
            return None;
        }
        let idx = (t * self.fps.max(1.0)).round() as u64;
        let img = self.frames.get(&idx)?.clone();
        if self.shown != Some((self.epoch, idx)) {
            let opts = egui::TextureOptions::LINEAR;
            match &mut self.tex {
                Some(t) => t.set(img, opts),
                None => self.tex = Some(ctx.load_texture("play-cache", img, opts)),
            }
            self.shown = Some((self.epoch, idx));
        }
        self.tex.as_ref().map(|t| t.id())
    }

    /// Cached frame ranges (first, last), for the timeline's bar.
    pub fn ranges(&self) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = Vec::new();
        for &i in self.frames.keys() {
            match out.last_mut() {
                Some(r) if r.1 + 1 == i => r.1 = i,
                _ => out.push((i, i)),
            }
        }
        out
    }

    /// A camera or the export frame has been filmed unchanged for `secs`: it
    /// is set up, so frames may be made while the user works elsewhere (Ash,
    /// 10-21). The canvas view never counts (every pan or zoom loses it).
    pub fn region_settled(&self, secs: f64) -> bool {
        matches!(self.target, Some(Target::Camera(_) | Target::Frame(_))) && self.since.elapsed().as_secs_f64() >= secs
    }

    pub fn fps(&self) -> f64 {
        self.fps
    }

    /// Frame size in pixels (a region's aspect).
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

/// Frame size for a target: the view's pixels, or the region's own pixels
/// (at time 0 for a camera), the long side at most `MAX_SIDE` either way (a
/// view bigger than that is shown stretched while playing).
fn frame_size(doc: &Document, target: Target) -> (u32, u32) {
    let region = match target {
        Target::View(v) => v.size * v.scale,
        Target::Frame(r) => r.size(),
        Target::Camera(id) => {
            let mut d = doc.clone();
            d.timeline.on = true;
            d.timeline.time = 0.0;
            crate::anim::evaluate(&mut d);
            match d.cameras.iter().find(|c| c.id == id) {
                Some(c) => c.rect.size(),
                None => return (0, 0),
            }
        }
    };
    let k = (MAX_SIDE / region.x.max(region.y).max(1.0)).min(1.0);
    ((region.x * k).round().max(2.0) as u32, (region.y * k).round().max(2.0) as u32)
}

/// Where a region's frame sits on the canvas at the posed document's time:
/// centre, size (canvas units), turn.
pub fn placement(doc: &Document, target: Target, size: (u32, u32)) -> Option<(Pos2, Vec2, f32)> {
    let (rect, rot) = match target {
        Target::View(v) => return Some((v.center, v.size, 0.0)),
        Target::Frame(r) => (r, 0.0),
        Target::Camera(id) => doc.cameras.iter().find(|c| c.id == id).map(|c| (c.rect, c.rot))?,
    };
    let r = crate::vexport::fit_aspect(rect, size.0 as f32 / size.1.max(1) as f32);
    Some((r.center(), r.size(), rot))
}

fn frame_bytes(f: &ColorImage) -> usize {
    f.pixels.len() * 4
}

/// (cap, low): the cache may use up to 75 % of physical memory, and gives frames
/// back while the system has less than 1 GB free.
fn budget() -> (usize, bool) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut m) } != 0 {
            // decoded video frames (`vcache`) share the 75 %
            let cap = (m.ullTotalPhys as f64 * 0.75) as usize;
            return (cap.saturating_sub(crate::vcache::bytes()), m.ullAvailPhys < (1u64 << 30));
        }
    }
    (4 << 30, false)
}

/// A layer's fingerprint: `full` covers everything a frame of it shows; its
/// place in the order, and its keys one by one (time, the values it holds, a
/// hash of it) and as they are, to tell an edit of keys alone. Masks never
/// count as keys alone: their keys reach their owner's frames.
struct Print {
    full: u64,
    span: Option<[f64; 2]>,
    index: usize,
    mask: bool,
    keys: Vec<(f64, u16, u64)>,
    anim_keys: Vec<crate::anim::Key>,
}

/// The layer at time 0 (its keys applied), as `fingerprints` takes it.
fn at_zero(l: &crate::doc::Layer) -> crate::doc::Layer {
    let mut x = l.clone();
    let base = crate::anim::key_of(&x, 0.0);
    if let Some(k) = x.anim.as_ref().and_then(|a| a.sample_on(0.0, &base)) {
        crate::anim::apply_key(&mut x, &k);
    }
    x
}

/// When only a layer's keys changed (`now`: the layer as committed, with its
/// old keys put back it prints as before): the time range they change, from
/// the key before each changed one to the key after it, per value it holds (a
/// value is eased between its own keys and held before the first and after
/// the last), cut to the layer's span. None when more than keys changed.
fn keys_changed(a: &Print, b: &Print, now: &crate::doc::Layer) -> Option<[f64; 2]> {
    if a.mask || b.mask || a.index != b.index || a.span != b.span {
        return None;
    }
    let mut old = now.clone();
    old.anim.as_mut()?.keys = a.anim_keys.clone();
    if layer_print(a.index, &at_zero(&old)) != a.full {
        return None;
    }
    let same = |t: f64, k: &(f64, u16, u64)| (k.0 - t).abs() < 1e-9;
    let all: Vec<&(f64, u16, u64)> = a.keys.iter().chain(&b.keys).collect();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for k in &all {
        let (ka, kb) = (a.keys.iter().find(|x| same(k.0, x)), b.keys.iter().find(|x| same(k.0, x)));
        if ka.is_some() && ka == kb {
            continue;
        }
        let ch = ka.map_or(0, |x| x.1) | kb.map_or(0, |x| x.1);
        for bit in (0..16).filter(|i| ch & (1 << i) != 0) {
            let near = |up: bool| {
                all.iter()
                    .filter(|x| x.1 & (1 << bit) != 0 && if up { x.0 > k.0 + 1e-9 } else { x.0 < k.0 - 1e-9 })
                    .map(|x| x.0)
                    .fold(if up { f64::INFINITY } else { f64::NEG_INFINITY }, |m, t| if up { m.min(t) } else { m.max(t) })
            };
            lo = lo.min(near(false));
            hi = hi.max(near(true));
        }
    }
    if lo > hi {
        return Some([0.0, -1.0]);
    }
    let [s0, s1] = a.span.unwrap_or([f64::NEG_INFINITY, f64::INFINITY]);
    Some([lo.max(s0), hi.min(s1)])
}

/// Per-layer fingerprints of what a frame shows (time-independent: keyed values
/// are taken at time 0, so moving the playhead changes nothing), and one for
/// everything shared (spaces, order).
fn fingerprints(doc: &Document) -> (HashMap<LayerId, Print>, u64) {
    let mut d = doc.clone();
    d.timeline.on = true;
    d.timeline.time = 0.0;
    crate::anim::evaluate(&mut d);
    let mut out = HashMap::new();
    for (i, (l, raw)) in d.layers.iter().zip(&doc.layers).enumerate() {
        let anim_keys = raw.anim.as_ref().map_or(Vec::new(), |a| a.keys.clone());
        let keys = anim_keys
            .iter()
            .map(|k| {
                let mut kh = std::collections::hash_map::DefaultHasher::new();
                json(k).hash(&mut kh);
                (k.t, k.ch, kh.finish())
            })
            .collect();
        // a mask (or its owner) can reach anywhere in time
        let span = if l.mask_of.is_some() { None } else { l.anim.as_ref().and_then(|a| a.span) };
        out.insert(l.id, Print { full: layer_print(i, l), span, index: i, mask: l.mask_of.is_some(), keys, anim_keys });
    }
    let mut g = std::collections::hash_map::DefaultHasher::new();
    json(&doc.spaces).hash(&mut g);
    json(&doc.cameras).hash(&mut g);
    json(&doc.export_frame).hash(&mut g);
    (out, g.finish())
}

/// What a frame of layer `l` (at index `i`) shows.
fn layer_print(i: usize, l: &crate::doc::Layer) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    i.hash(&mut h);
    match &l.kind {
        // a video's frame changes with the playhead: its file and settings count
        LayerKind::Image(im) if im.video.is_some() => {
            let v = im.video.as_ref().unwrap();
            (&v.path, v.speed.to_bits(), v.looping, im.adjust.key()).hash(&mut h);
            for x in [im.crop.min.x, im.crop.min.y, im.crop.max.x, im.crop.max.y, im.scale, im.rotation, im.center.x, im.center.y] {
                x.to_bits().hash(&mut h);
            }
        }
        _ => crate::canvas::content_key(l).hash(&mut h),
    }
    match &l.kind {
        LayerKind::Text(t) => (t.blend as u8).hash(&mut h),
        LayerKind::Raster(r) => (r.blend as u8).hash(&mut h),
        _ => {}
    }
    (l.opacity.to_bits(), l.visible, l.mask_of, l.mask_invert).hash(&mut h);
    json(&l.anim).hash(&mut h);
    json(&l.place).hash(&mut h);
    json(&l.mask_anchor).hash(&mut h);
    json(&l.mask_pose).hash(&mut h);
    h.finish()
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn worker(rx: mpsc::Receiver<Job>, tx: mpsc::Sender<Msg>, ctx: egui::Context) {
    let mut job: Option<Job> = None;
    let mut feeds: HashMap<LayerId, crate::vexport::Feed> = HashMap::new();
    let mut feeds_of: usize = 0;
    loop {
        if job.is_none() {
            match rx.recv() {
                Ok(j) => job = Some(j),
                Err(_) => return,
            }
        }
        while let Ok(j) = rx.try_recv() {
            job = Some(j);
        }
        let j = job.as_mut().unwrap();
        if j.frames.is_empty() {
            let _ = tx.send(Msg::Done { epoch: j.epoch, ok: true });
            job = None;
            continue;
        }
        let doc_ptr = Arc::as_ptr(&j.doc) as usize;
        if doc_ptr != feeds_of {
            feeds = crate::vexport::feeds_for(&j.doc, true);
            feeds_of = doc_ptr;
        }
        let idx = j.frames.remove(0);
        let t = idx as f64 / j.fps;
        let mut d = crate::vexport::pose_keys(&j.doc, t);
        let Some((center, size, rot)) = placement(&d, j.target, j.size) else {
            let _ = tx.send(Msg::Done { epoch: j.epoch, ok: true });
            job = None;
            continue;
        };
        let scale = j.size.0 as f32 / size.x.max(1e-3);
        // video frames from the video cache where its preview size is enough
        crate::vexport::fill_videos(&mut d, t, &mut feeds, Some(scale));
        let r = crate::gpu::with_gpu(|g| g.render_ss(&d, center, size, rot, scale, None, 1));
        match r {
            Some(Ok(pm)) => {
                let img = ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
                if tx.send(Msg::Frame { epoch: j.epoch, idx, img: Arc::new(img) }).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            _ => {
                let _ = tx.send(Msg::Done { epoch: j.epoch, ok: false });
                job = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{ShapeKind, ShapeLayer};
    use egui::{Color32, pos2, vec2};

    fn shape(doc: &mut Document, x: f32, span: Option<[f64; 2]>) -> LayerId {
        let s = ShapeLayer { kind: ShapeKind::Rect, a: pos2(x, 0.0), b: pos2(x + 10.0, 10.0), color: Color32::RED, width: 0.0, fill: Some(Color32::RED), rotation: 0.0 };
        let id = doc.insert("s".into(), LayerKind::Shape(s), None);
        if let Some(sp) = span {
            doc.get_mut(id).unwrap().anim = Some(crate::anim::Anim { span: Some(sp), ..Default::default() });
        }
        id
    }

    fn filled(c: &mut PlayCache, n: u64) {
        for i in 0..n {
            let img = Arc::new(ColorImage::new([2, 2], vec![Color32::BLACK; 4]));
            c.bytes += frame_bytes(&img);
            c.frames.insert(i, img);
        }
    }

    #[test]
    fn edits_drop_only_their_span() {
        let mut doc = Document::default();
        doc.timeline.fps = 10.0;
        doc.timeline.duration = 10.0;
        let a = shape(&mut doc, 0.0, Some([2.0, 4.0]));
        shape(&mut doc, 50.0, Some([6.0, 8.0]));
        let view = View { center: pos2(0.0, 0.0), size: vec2(100.0, 100.0), scale: 1.0 };
        let mut c = PlayCache::new();
        c.off = false;
        c.sync(1, &doc, Target::View(view));
        filled(&mut c, 100);
        // the playhead moving (keys evaluated at another time) is not an edit
        doc.timeline.time = 5.0;
        c.sync(2, &doc, Target::View(view));
        assert_eq!(c.frames.len(), 100);
        // a change to the first shape drops 2–4 s (with a frame either side)
        if let Some(LayerKind::Shape(s)) = doc.get_mut(a).map(|l| &mut l.kind) {
            s.fill = Some(Color32::BLUE);
        }
        c.sync(3, &doc, Target::View(view));
        assert!(!c.frames.contains_key(&30) && !c.frames.contains_key(&19) && !c.frames.contains_key(&41));
        assert!(c.frames.contains_key(&10) && c.frames.contains_key(&50) && c.frames.contains_key(&70));
        // a new layer with no span reaches everywhere
        shape(&mut doc, 90.0, None);
        c.sync(4, &doc, Target::View(view));
        assert!(c.frames.is_empty());
        // another view starts over
        filled(&mut c, 5);
        c.sync(4, &doc, Target::View(View { scale: 2.0, ..view }));
        assert!(c.frames.is_empty());
        // an export frame doesn't care about the view; its frames are its own
        // pixels, the long side at most 3840
        let fr = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(1080.0, 1920.0));
        c.sync(5, &doc, Target::Frame(fr));
        assert_eq!(c.size(), (1080, 1920));
        filled(&mut c, 5);
        c.sync(5, &doc, Target::Frame(fr));
        assert_eq!(c.frames.len(), 5);
        // the timeline made shorter or longer: what is cached stays (frames
        // past a shorter end too, for when it grows back)
        doc.timeline.duration = 0.3;
        c.sync(7, &doc, Target::Frame(fr));
        assert_eq!(c.frames.len(), 5);
        doc.timeline.duration = 20.0;
        c.sync(8, &doc, Target::Frame(fr));
        assert_eq!(c.frames.len(), 5);
        let big = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(7680.0, 4320.0));
        c.sync(6, &doc, Target::Frame(big));
        assert_eq!(c.size(), (3840, 2160));
        // a view that big too
        c.sync(9, &doc, Target::View(View { center: pos2(0.0, 0.0), size: vec2(3840.0, 2160.0), scale: 2.0 }));
        assert_eq!(c.size(), (3840, 2160));
    }

    #[test]
    fn a_key_edit_drops_only_between_its_neighbours() {
        use crate::anim::ch;
        let mut doc = Document::default();
        doc.timeline.fps = 10.0;
        doc.timeline.duration = 10.0;
        let a = shape(&mut doc, 0.0, None);
        // position keys at 1, 3 and 5 s, an opacity key at 2 s between them
        let l = doc.get(a).unwrap().clone();
        let key = |t: f64, c: u16, x: f32| {
            let mut k = crate::anim::key_of(&l, t);
            k.ch = c;
            k.center[0] += x;
            k
        };
        doc.get_mut(a).unwrap().anim = Some(crate::anim::Anim { keys: vec![key(1.0, ch::POS, 0.0), key(2.0, ch::OPA, 0.0), key(3.0, ch::POS, 20.0), key(5.0, ch::POS, 40.0)], ..Default::default() });
        let view = View { center: pos2(0.0, 0.0), size: vec2(100.0, 100.0), scale: 1.0 };
        let mut c = PlayCache::new();
        c.off = false;
        c.sync(1, &doc, Target::View(view));
        filled(&mut c, 100);
        // the 3 s position key moved: its position runs from the 1 s key to the
        // 5 s one (the opacity key at 2 s is no bound for it)
        doc.get_mut(a).unwrap().anim.as_mut().unwrap().keys[2].center[0] += 15.0;
        // (the stored layer holds its keyed values at the playhead, wherever
        // that was at the commit: they don't count)
        doc.timeline.on = true;
        doc.timeline.time = 4.2;
        crate::anim::evaluate(&mut doc);
        c.sync(2, &doc, Target::View(view));
        assert!(c.frames.contains_key(&8) && c.frames.contains_key(&52) && c.frames.contains_key(&90));
        assert!(!c.frames.contains_key(&10) && !c.frames.contains_key(&25) && !c.frames.contains_key(&50));
        // the first position key: held before it, so from the start to 3 s
        filled(&mut c, 100);
        doc.get_mut(a).unwrap().anim.as_mut().unwrap().keys[0].center[0] -= 5.0;
        c.sync(3, &doc, Target::View(view));
        assert!(!c.frames.contains_key(&0) && !c.frames.contains_key(&29));
        assert!(c.frames.contains_key(&32) && c.frames.contains_key(&90));
        // anything but keys (here the colour): the layer's whole span (all of it)
        filled(&mut c, 100);
        if let Some(LayerKind::Shape(s)) = doc.get_mut(a).map(|l| &mut l.kind) {
            s.fill = Some(Color32::BLUE);
        }
        c.sync(4, &doc, Target::View(view));
        assert!(c.frames.is_empty());
    }
}
