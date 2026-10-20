//! Playback cache (render overhaul phase 4, docs/updates/wgpu-renderer.md).
//!
//! Frames of the canvas as it is framed now (screen pixels), keyed by frame
//! number, rendered ahead by a worker thread on the GPU export renderer from the
//! last committed document. While the timeline plays, a cached frame is shown
//! in place of drawing the document live; frames that aren't cached yet are
//! drawn live as before. Filled ahead of the playhead while playing, and around
//! it after a short idle. RAM: as much as needed, up to 75 % of physical memory,
//! given back when less than 1 GB is free (Ash, 10-20).

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

struct Job {
    epoch: u64,
    doc: Arc<Document>,
    view: View,
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
    view: Option<View>,
    fps: f64,
    duration: f64,
    /// Per layer: (fingerprint, the time span it shows in).
    revs: HashMap<LayerId, (u64, Option<[f64; 2]>)>,
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
}

impl PlayCache {
    pub fn new() -> Self {
        PlayCache {
            frames: BTreeMap::new(),
            bytes: 0,
            epoch: 0,
            view: None,
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
        }
    }

    fn clear(&mut self) {
        self.frames.clear();
        self.bytes = 0;
        self.epoch += 1;
        self.shown = None;
    }

    /// Follows the committed document (`rev` changes on every commit, undo and
    /// load) and the view: changed layers drop the frames of their time span; a
    /// new view, fps or space drops everything.
    pub fn sync(&mut self, rev: u64, committed: &Document, view: View) {
        if self.off {
            return;
        }
        let tl = &committed.timeline;
        if self.view != Some(view) || self.fps != tl.fps {
            self.view = Some(view);
            self.fps = tl.fps;
            self.clear();
        }
        self.duration = tl.duration;
        if self.doc_rev == Some(rev) {
            return;
        }
        self.doc_rev = Some(rev);
        self.doc = Some(Arc::new(committed.clone()));
        let (revs, global) = fingerprints(committed);
        if global != self.global {
            self.global = global;
            self.revs = revs;
            self.clear();
            return;
        }
        // spans of layers that changed, appeared or went away
        let mut spans: Vec<Option<[f64; 2]>> = Vec::new();
        for (id, (r, span)) in &revs {
            match self.revs.get(id) {
                Some((r0, span0)) if r0 == r => {}
                Some((_, span0)) => {
                    spans.push(*span0);
                    spans.push(*span);
                }
                None => spans.push(*span),
            }
        }
        for (id, (_, span0)) in &self.revs {
            if !revs.contains_key(id) {
                spans.push(*span0);
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
        if self.off || self.view.is_none() || self.doc.is_none() {
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
        let (Some(tx), Some(doc), Some(view)) = (&self.tx, &self.doc, self.view) else { return };
        let _ = tx.send(Job { epoch: self.epoch, doc: doc.clone(), view, fps, frames: list.clone() });
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

    pub fn fps(&self) -> f64 {
        self.fps
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
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
            return ((m.ullTotalPhys as f64 * 0.75) as usize, m.ullAvailPhys < (1u64 << 30));
        }
    }
    (4 << 30, false)
}

/// Per-layer fingerprints of what a frame shows (time-independent: keyed values
/// are taken at time 0, so moving the playhead changes nothing), and one for
/// everything shared (spaces, order).
fn fingerprints(doc: &Document) -> (HashMap<LayerId, (u64, Option<[f64; 2]>)>, u64) {
    let mut d = doc.clone();
    d.timeline.on = true;
    d.timeline.time = 0.0;
    crate::anim::evaluate(&mut d);
    let mut out = HashMap::new();
    for (i, l) in d.layers.iter().enumerate() {
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
        // a mask (or its owner) can reach anywhere in time
        let span = if l.mask_of.is_some() { None } else { l.anim.as_ref().and_then(|a| a.span) };
        out.insert(l.id, (h.finish(), span));
    }
    let mut g = std::collections::hash_map::DefaultHasher::new();
    json(&doc.spaces).hash(&mut g);
    (out, g.finish())
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
        let d = crate::vexport::pose_at(&j.doc, t, &mut feeds);
        let v = j.view;
        let r = crate::gpu::with_gpu(|g| g.render_ss(&d, v.center, v.size, 0.0, v.scale, None, 1));
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
        c.sync(1, &doc, view);
        filled(&mut c, 100);
        // the playhead moving (keys evaluated at another time) is not an edit
        doc.timeline.time = 5.0;
        c.sync(2, &doc, view);
        assert_eq!(c.frames.len(), 100);
        // a change to the first shape drops 2–4 s (with a frame either side)
        if let Some(LayerKind::Shape(s)) = doc.get_mut(a).map(|l| &mut l.kind) {
            s.fill = Some(Color32::BLUE);
        }
        c.sync(3, &doc, view);
        assert!(!c.frames.contains_key(&30) && !c.frames.contains_key(&19) && !c.frames.contains_key(&41));
        assert!(c.frames.contains_key(&10) && c.frames.contains_key(&50) && c.frames.contains_key(&70));
        // a new layer with no span reaches everywhere
        shape(&mut doc, 90.0, None);
        c.sync(4, &doc, view);
        assert!(c.frames.is_empty());
        // another view starts over
        filled(&mut c, 5);
        c.sync(4, &doc, View { scale: 2.0, ..view });
        assert!(c.frames.is_empty());
    }
}
