// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (AGPL-3.0 section 7): see NOTICE.

//! Decoded video frames kept per clip (Ash, 10-21): a background decoder fills
//! the part of each video the timeline shows, at preview size (long side 1920),
//! as NV12 (RGBA for transparent ones: GIFs), whatever is being filmed.
//! Scrubbing and starting playback then take
//! frames from memory instead of a fresh ffmpeg seek each (0.4–0.6 s for an FHD
//! clip). Up to 30 % of physical memory; the playback cache's share shrinks by
//! what this holds.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use crate::doc::Nv12;

/// What one video layer needs: its file, frame rate, preview size, the
/// frames (first, last) its clip shows, and whether it is transparent.
#[derive(Clone, PartialEq, Debug)]
pub struct Want {
    pub path: String,
    pub fps: f64,
    pub size: (u32, u32),
    pub range: (u64, u64),
    pub alpha: bool,
}

/// A held frame: NV12, or RGBA (straight alpha) for a transparent video.
#[derive(Clone)]
pub enum Frame {
    Nv12(Arc<Nv12>),
    Rgba(Arc<image::RgbaImage>),
}

impl Frame {
    pub fn size(&self) -> (u32, u32) {
        match self {
            Frame::Nv12(f) => (f.w, f.h),
            Frame::Rgba(i) => i.dimensions(),
        }
    }

    fn bytes(&self) -> usize {
        match self {
            Frame::Nv12(f) => f.data.len(),
            Frame::Rgba(i) => i.as_raw().len(),
        }
    }

    /// As RGBA (an NV12 frame converted on every core).
    pub fn to_rgba(&self) -> image::RgbaImage {
        match self {
            Frame::Nv12(f) => to_rgba(f),
            Frame::Rgba(i) => (**i).clone(),
        }
    }

    /// The frame as an image source whose geometry is the video's `logical`
    /// size (the GPU takes NV12 as it is).
    pub fn source(&self, logical: [u32; 2]) -> crate::doc::ImageSource {
        let (rgba, nv12) = match self {
            Frame::Nv12(f) => (Arc::new(image::RgbaImage::new(0, 0)), Some(f.clone())),
            Frame::Rgba(i) => (i.clone(), None),
        };
        crate::doc::ImageSource { id: crate::doc::next_rev(), rgba, encoded: Arc::new(Vec::new()), ext: "png".into(), logical: Some(logical), nv12 }
    }
}

#[derive(Default)]
struct Clip {
    fps: f64,
    size: (u32, u32),
    frames: BTreeMap<u64, Frame>,
}

#[derive(Default)]
struct State {
    clips: HashMap<String, Clip>,
    wants: Vec<Want>,
    /// Bumped when the wants change (the decoder starts over).
    epoch: u64,
    bytes: usize,
    started: bool,
    /// Per clip, the frame where decoding gave nothing (past the file's end
    /// or a broken stream): not tried again from there.
    dead: HashMap<String, u64>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn cap() -> usize {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut m) } != 0 {
            // and never while the system has less than 1.5 GB free
            if m.ullAvailPhys < (3u64 << 29) {
                return 0;
            }
            return (m.ullTotalPhys as f64 * 0.3) as usize;
        }
    }
    1 << 30
}

/// The clips to keep and fill (called every frame; cheap when unchanged).
/// Clips no longer wanted are dropped; a clip wanted at another size or rate
/// starts over.
pub fn want(list: Vec<Want>) {
    if std::env::var_os("DV_NO_VCACHE").is_some() {
        return;
    }
    let mut s = state().lock().unwrap();
    if s.wants == list {
        return;
    }
    s.clips.retain(|p, c| list.iter().any(|w| &w.path == p && w.size == c.size && w.fps == c.fps));
    // a clip no longer wanted may come back as another file under its path
    s.dead.retain(|p, _| list.iter().any(|w| &w.path == p));
    s.bytes = s.clips.values().flat_map(|c| c.frames.values()).map(|f| f.bytes()).sum();
    s.wants = list;
    s.epoch += 1;
    if !s.started {
        s.started = true;
        std::thread::spawn(filler);
    }
}

/// The frame at video time `t` of `path`, when it is held.
pub fn get(path: &str, t: f64) -> Option<Frame> {
    let s = state().lock().unwrap();
    let c = s.clips.get(path)?;
    c.frames.get(&((t * c.fps).round().max(0.0) as u64)).cloned()
}

/// Whether frames `t0`..`t0 + secs` of `path` are all held.
pub fn holds(path: &str, t0: f64, secs: f64) -> bool {
    let s = state().lock().unwrap();
    let Some(c) = s.clips.get(path) else { return false };
    let a = (t0 * c.fps).round().max(0.0) as u64;
    let b = ((t0 + secs) * c.fps).round() as u64;
    (a..=b).all(|i| c.frames.contains_key(&i))
}

/// Held frames of `path` as ranges (first, last frame), with the clip's rate.
pub fn ranges(path: &str) -> (Vec<(u64, u64)>, f64) {
    let s = state().lock().unwrap();
    let Some(c) = s.clips.get(path) else { return (Vec::new(), 1.0) };
    let mut out: Vec<(u64, u64)> = Vec::new();
    for &i in c.frames.keys() {
        match out.last_mut() {
            Some(r) if r.1 + 1 == i => r.1 = i,
            _ => out.push((i, i)),
        }
    }
    (out, c.fps.max(1.0))
}

/// The decoder is filling (the timeline's clip bars change).
pub fn busy() -> bool {
    BUSY.load(std::sync::atomic::Ordering::Relaxed)
}

static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Memory held, bytes.
pub fn bytes() -> usize {
    state().lock().map_or(0, |s| s.bytes)
}

/// Frames held (for `DV_TIMING`).
pub fn held() -> usize {
    state().lock().map_or(0, |s| s.clips.values().map(|c| c.frames.len()).sum())
}

/// Scrubs served from memory and by a seek, and playback cache frames that
/// took their video frame from here (for `DV_TIMING`).
pub static SCRUBS: [std::sync::atomic::AtomicU64; 3] = [std::sync::atomic::AtomicU64::new(0), std::sync::atomic::AtomicU64::new(0), std::sync::atomic::AtomicU64::new(0)];

/// The decoder: the first frame some clip wants and lacks, decoded on from
/// there in one ffmpeg stream until the clip's range ends, a frame already held
/// comes up, the wants change or memory runs out.
fn filler() {
    let hwaccel = if crate::gpu::with_gpu(|g| g.vendor) == Some(0x10de) { "cuda" } else { "auto" };
    loop {
        let next = {
            let mut s = state().lock().unwrap();
            let epoch = s.epoch;
            let room = s.bytes < cap();
            let mut pick = None;
            if room {
                for w in s.wants.clone() {
                    if s.dead.get(&w.path).is_some_and(|&d| d <= w.range.0) {
                        continue;
                    }
                    let end = s.dead.get(&w.path).map_or(w.range.1, |d| w.range.1.min(d.saturating_sub(1)));
                    let c = s.clips.entry(w.path.clone()).or_insert_with(|| Clip { fps: w.fps, size: w.size, frames: BTreeMap::new() });
                    if let Some(i) = (w.range.0..=end).find(|i| !c.frames.contains_key(i)) {
                        pick = Some((w, i, epoch));
                        break;
                    }
                }
            }
            pick
        };
        BUSY.store(next.is_some(), std::sync::atomic::Ordering::Relaxed);
        let Some((w, first, epoch)) = next else {
            std::thread::sleep(std::time::Duration::from_millis(200));
            continue;
        };
        let start = if w.alpha { crate::video::Player::start_rgba } else { crate::video::Player::start_nv12 };
        let Ok(player) = start(std::path::Path::new(&w.path), first as f64 / w.fps, w.fps, w.size, hwaccel) else {
            std::thread::sleep(std::time::Duration::from_millis(1000));
            continue;
        };
        let mut idx = first;
        let mut got = 0;
        // stopped by us (the wants changed, memory is full), not by the file
        let mut cut = false;
        while let Ok((_, data)) = player.rx.recv_timeout(std::time::Duration::from_secs(5)) {
            let mut s = state().lock().unwrap();
            if s.epoch != epoch || s.bytes >= cap() {
                cut = true;
                break;
            }
            let n = data.len();
            let c = s.clips.entry(w.path.clone()).or_default();
            if c.frames.contains_key(&idx) {
                break;
            }
            let f = match w.alpha {
                true => match image::RgbaImage::from_raw(w.size.0, w.size.1, data) {
                    Some(i) => Frame::Rgba(Arc::new(i)),
                    None => break,
                },
                false => Frame::Nv12(Arc::new(Nv12 { w: w.size.0, h: w.size.1, data })),
            };
            c.frames.insert(idx, f);
            s.bytes += n;
            got += 1;
            idx += 1;
            if idx > w.range.1 {
                break;
            }
        }
        // no frame from there on: the file ends there. Not when we stopped
        // before taking one (memory short for a moment): that left the clip
        // uncached from this frame on until the app closed.
        if got == 0 && !cut {
            let mut s = state().lock().unwrap();
            if s.epoch == epoch {
                let d = s.dead.entry(w.path.clone()).or_insert(u64::MAX);
                *d = (*d).min(first);
            }
        }
    }
}

/// NV12 to RGBA on every core (a cached frame shown on the canvas).
pub fn to_rgba(f: &Nv12) -> image::RgbaImage {
    let (w, h) = (f.w as usize, f.h as usize);
    let (ys, uvs) = f.data.split_at(w * h);
    let mut out = vec![255u8; w * h * 4];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = h.div_ceil(threads).max(2) / 2 * 2;
    std::thread::scope(|sc| {
        for (k, part) in out.chunks_mut(rows * w * 4).enumerate() {
            sc.spawn(move || {
                for (r, row) in part.chunks_mut(w * 4).enumerate() {
                    let y = k * rows + r;
                    for x in 0..w {
                        let yy = (ys[y * w + x] as f32 - 16.0) / 219.0;
                        let i = (y / 2) * w + (x / 2) * 2;
                        let cb = (uvs[i] as f32 - 128.0) / 224.0;
                        let cr = (uvs[i + 1] as f32 - 128.0) / 224.0;
                        let rgb = [yy + 1.5748 * cr, yy - 0.187324 * cb - 0.468124 * cr, yy + 1.8556 * cb];
                        for (c, v) in rgb.into_iter().enumerate() {
                            row[x * 4 + c] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            });
        }
    });
    image::RgbaImage::from_raw(f.w, f.h, out).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_matches_the_reference() {
        let (w, h) = (6u32, 4u32);
        let data: Vec<u8> = (0..(w * h + w * h / 2)).map(|i| (i * 37 % 251) as u8).collect();
        let f = Nv12 { w, h, data };
        assert_eq!(to_rgba(&f), f.to_rgba());
    }
}
