//! Video export (docs/VIDEO.md, V4): renders the document over a time range and
//! pipes the frames into ffmpeg, on a worker thread with progress and cancel.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use egui::{Color32, Rect, vec2};
use serde::{Deserialize, Serialize};

use crate::doc::{Document, ImageSource, LayerId, LayerKind, next_rev};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
    ProRes,
    Vp9,
    Gif,
    Png,
}

impl Codec {
    pub const ALL: [Codec; 7] = [Codec::H264, Codec::Hevc, Codec::Av1, Codec::ProRes, Codec::Vp9, Codec::Gif, Codec::Png];

    pub fn label(self) -> &'static str {
        match self {
            Codec::H264 => "H.264 (mp4)",
            Codec::Hevc => "H.265/HEVC (mp4)",
            Codec::Av1 => "AV1 (mp4)",
            Codec::ProRes => "ProRes 4444 (mov, 투명)",
            Codec::Vp9 => "VP9 (webm, 투명)",
            Codec::Gif => "GIF",
            Codec::Png => "PNG 연속 이미지 (투명)",
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Codec::H264 | Codec::Hevc | Codec::Av1 => "mp4",
            Codec::ProRes => "mov",
            Codec::Vp9 => "webm",
            Codec::Gif => "gif",
            Codec::Png => "png",
        }
    }

    pub fn alpha(self) -> bool {
        matches!(self, Codec::ProRes | Codec::Vp9 | Codec::Png)
    }

    pub fn audio(self) -> bool {
        !matches!(self, Codec::Gif | Codec::Png)
    }

    /// Uses a quality number (CRF-like, lower is better).
    pub fn has_quality(self) -> bool {
        matches!(self, Codec::H264 | Codec::Hevc | Codec::Av1 | Codec::Vp9)
    }

    pub fn has_gpu(self) -> bool {
        matches!(self, Codec::H264 | Codec::Hevc | Codec::Av1)
    }

    /// Needs even width and height (4:2:0).
    pub fn even(self) -> bool {
        matches!(self, Codec::H264 | Codec::Hevc | Codec::Av1 | Codec::Vp9)
    }

    fn args(self, quality: u8, mbps: Option<f64>, gpu: bool) -> Vec<String> {
        let q = quality.to_string();
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let mut a = match (self, gpu) {
            (Codec::H264, false) => v(&["-c:v", "libx264", "-preset", "medium", "-pix_fmt", "yuv420p"]),
            (Codec::H264, true) => v(&["-c:v", "h264_nvenc", "-preset", "p5", "-pix_fmt", "yuv420p"]),
            (Codec::Hevc, false) => v(&["-c:v", "libx265", "-preset", "medium", "-pix_fmt", "yuv420p", "-tag:v", "hvc1"]),
            (Codec::Hevc, true) => v(&["-c:v", "hevc_nvenc", "-preset", "p5", "-pix_fmt", "yuv420p", "-tag:v", "hvc1"]),
            (Codec::Av1, false) => v(&["-c:v", "libsvtav1", "-preset", "8", "-pix_fmt", "yuv420p"]),
            (Codec::Av1, true) => v(&["-c:v", "av1_nvenc", "-preset", "p5", "-pix_fmt", "yuv420p"]),
            (Codec::ProRes, _) => v(&["-c:v", "prores_ks", "-profile:v", "4", "-pix_fmt", "yuva444p10le"]),
            (Codec::Vp9, _) => v(&["-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-row-mt", "1", "-deadline", "good", "-cpu-used", "4"]),
            (Codec::Gif, _) => v(&["-vf", "split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer", "-loop", "0"]),
            (Codec::Png, _) => Vec::new(),
        };
        if self.has_quality() {
            match mbps {
                Some(m) => a.extend(v(&["-b:v", &format!("{m}M")])),
                None if gpu => a.extend(v(&["-cq", &q, "-b:v", "0"])),
                None if self == Codec::Vp9 => a.extend(v(&["-crf", &q, "-b:v", "0"])),
                None => a.extend(v(&["-crf", &q])),
            }
        }
        if matches!(self, Codec::H264 | Codec::Hevc | Codec::Av1 | Codec::ProRes) {
            a.extend(v(&["-movflags", "+faststart"]));
        }
        a
    }

    fn audio_args(self) -> Vec<String> {
        match self {
            Codec::Vp9 => vec!["-c:a".into(), "libopus".into(), "-b:a".into(), "160k".into()],
            _ => vec!["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()],
        }
    }
}

/// What the video films: a canvas camera (moving with its keys) or a fixed rect.
#[derive(Clone, Copy, Debug)]
pub enum Region {
    Camera(u64),
    Fixed(Rect),
}

pub struct Job {
    pub doc: Document,
    pub region: Region,
    pub size: (u32, u32),
    pub fps: f64,
    pub t0: f64,
    pub t1: f64,
    pub codec: Codec,
    pub quality: u8,
    pub mbps: Option<f64>,
    pub gpu: bool,
    /// None: transparent (alpha codecs) or black.
    pub bg: Option<Color32>,
    pub audio: Vec<crate::audio::Voice>,
    /// A file, or for PNG sequences the first file's path (`name_00001.png` …).
    pub out: PathBuf,
}

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
    /// Set when finished: Ok(message) or Err(reason).
    pub result: Mutex<Option<Result<String, String>>>,
}

/// Fits `r` to the aspect ratio `ar` (width / height) by growing it around its centre.
pub fn fit_aspect(r: Rect, ar: f32) -> Rect {
    let (w, h) = (r.width().max(1.0), r.height().max(1.0));
    let size = if w / h > ar { vec2(w, w / ar) } else { vec2(h * ar, h) };
    Rect::from_center_size(r.center(), size)
}

/// Output size for a region aspect, from a width or a height (even when `even`).
pub fn size_for(aspect: f32, by_width: bool, n: u32, even: bool) -> (u32, u32) {
    let round = |x: f32| {
        let x = x.round().max(2.0) as u32;
        if even { x + (x & 1) } else { x }
    };
    let n = round(n as f32);
    if by_width { (n, round(n as f32 / aspect.max(1e-3))) } else { (round(n as f32 * aspect), n) }
}

pub fn start(job: Job) -> Arc<Progress> {
    let p = Arc::new(Progress::default());
    let total = (((job.t1 - job.t0) * job.fps).ceil().max(1.0)) as u64;
    p.total.store(total, Ordering::Relaxed);
    let pr = p.clone();
    std::thread::spawn(move || {
        let r = run(&job, &pr, total);
        if let Ok(mut g) = pr.result.lock() {
            *g = Some(r);
        }
    });
    p
}

/// Sequential full-resolution frames of one video layer.
struct Feed {
    path: PathBuf,
    fps: f64,
    size: (u32, u32),
    player: Option<crate::video::Player>,
    last: f64,
    cur: Option<(f64, Arc<image::RgbaImage>)>,
    pending: Option<(f64, image::RgbaImage)>,
}

impl Feed {
    fn frame(&mut self, vt: f64) -> Option<Arc<image::RgbaImage>> {
        let ahead = self.cur.as_ref().is_some_and(|c| vt > c.0 + 2.0);
        if self.player.is_none() || vt < self.last - 1e-6 || ahead {
            self.player = crate::video::Player::start(&self.path, vt, self.fps, self.size).ok();
            self.cur = None;
            self.pending = None;
        }
        self.last = vt;
        let half = 0.5 / self.fps.max(1.0);
        loop {
            if self.pending.is_none() {
                match self.player.as_ref()?.rx.recv_timeout(std::time::Duration::from_secs(10)) {
                    Ok(f) => self.pending = Some(f),
                    Err(_) => break,
                }
            }
            if self.pending.as_ref().is_some_and(|(t, _)| *t <= vt + half) {
                let (t, img) = self.pending.take().unwrap();
                self.cur = Some((t, Arc::new(img)));
            } else {
                break;
            }
        }
        self.cur.as_ref().map(|c| c.1.clone())
    }
}

fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// 16-bit stereo WAV of the mixed voices over [t0, t1).
fn write_wav(path: &Path, voices: &[crate::audio::Voice], t0: f64, t1: f64) -> std::io::Result<()> {
    let rate = 48_000u32;
    let n = ((t1 - t0) * rate as f64).round() as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let data_len = n * 4;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * 4).to_le_bytes())?;
    f.write_all(&4u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for i in 0..n {
        let (l, r) = crate::audio::mix_at(voices, t0 + i as f64 / rate as f64);
        f.write_all(&((l * 32767.0) as i16).to_le_bytes())?;
        f.write_all(&((r * 32767.0) as i16).to_le_bytes())?;
    }
    f.flush()
}

fn run(job: &Job, pr: &Progress, total: u64) -> Result<String, String> {
    let (w, h) = job.size;
    let tools = crate::video::tools().ok_or(crate::video::MISSING)?;
    // sound first (a temp WAV the encoder muxes in)
    let wav = (job.codec.audio() && !job.audio.is_empty()).then(|| std::env::temp_dir().join(format!("dv_export_{}.wav", next_rev())));
    if let Some(p) = &wav {
        write_wav(p, &job.audio, job.t0, job.t1).map_err(|e| format!("소리 준비 실패: {e}"))?;
    }
    let cleanup = || {
        if let Some(p) = &wav {
            let _ = std::fs::remove_file(p);
        }
    };
    let mut child = None;
    if job.codec != Codec::Png {
        let mut cmd = Command::new(&tools.ffmpeg);
        cmd.args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-r", &format!("{}", job.fps), "-i", "pipe:0"]);
        if let Some(p) = &wav {
            cmd.arg("-i").arg(p);
        }
        cmd.args(job.codec.args(job.quality, job.mbps, job.gpu));
        if wav.is_some() {
            cmd.args(job.codec.audio_args());
            cmd.arg("-shortest");
        }
        cmd.arg(&job.out).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        child = Some(no_window(&mut cmd).spawn().map_err(|e| e.to_string())?);
    }
    // video layers: one sequential decoder each
    let mut feeds: std::collections::HashMap<LayerId, Feed> = std::collections::HashMap::new();
    for l in &job.doc.layers {
        if let LayerKind::Image(im) = &l.kind {
            if let Some(v) = &im.video {
                feeds.insert(l.id, Feed { path: PathBuf::from(&v.path), fps: v.info.fps, size: (v.info.width, v.info.height), player: None, last: f64::MIN, cur: None, pending: None });
            }
        }
    }
    let png_base = job.out.with_extension("");
    let fail = |child: &mut Option<std::process::Child>, msg: String| {
        if let Some(c) = child.as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        cleanup();
        Err(msg)
    };
    for i in 0..total {
        if pr.cancel.load(Ordering::Relaxed) {
            return fail(&mut child, "취소했어".into());
        }
        let t = job.t0 + i as f64 / job.fps;
        let mut d = job.doc.clone();
        d.timeline.on = true;
        d.timeline.time = t;
        crate::anim::evaluate(&mut d);
        crate::persp::sync_walls(&mut d);
        // video frames at full resolution
        let alive: Vec<bool> = d.layers.iter().map(|l| d.alive(l)).collect();
        for (l, alive) in d.layers.iter_mut().zip(alive) {
            if !alive {
                continue;
            }
            let anim = l.anim.clone();
            if let LayerKind::Image(im) = &mut l.kind {
                if let (Some(v), Some(feed)) = (im.video.as_ref(), feeds.get_mut(&l.id)) {
                    let vt = crate::anim::video_time(anim.as_ref(), v.speed, t).clamp(0.0, (v.info.duration - v.info.frame_len()).max(0.0));
                    if let Some(img) = feed.frame(vt) {
                        im.src = ImageSource { id: next_rev(), rgba: img, encoded: Arc::new(Vec::new()), ext: "png".into(), logical: None };
                    }
                }
            }
        }
        let (region, rot) = match job.region {
            Region::Camera(id) => match d.cameras.iter().find(|c| c.id == id) {
                Some(c) => (c.rect, c.rot),
                None => return fail(&mut child, "카메라가 없어".into()),
            },
            Region::Fixed(r) => (r, 0.0),
        };
        let r = fit_aspect(region, w as f32 / h as f32);
        let scale = w as f32 / r.width();
        let size = vec2(w as f32 / scale, h as f32 / scale);
        let bg = if job.codec.alpha() { job.bg } else { Some(job.bg.unwrap_or(Color32::BLACK)) };
        let pm = match crate::export::render_rotated(&d, r.center(), size, rot, scale, bg) {
            Ok(pm) => pm,
            Err(e) => return fail(&mut child, e),
        };
        let mut rgba = crate::export::to_rgba(&pm);
        if rgba.dimensions() != (w, h) {
            rgba = image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Triangle);
        }
        match child.as_mut() {
            Some(c) => {
                let ok = c.stdin.as_mut().is_some_and(|s| s.write_all(rgba.as_raw()).is_ok());
                if !ok {
                    let mut err = String::new();
                    if let Some(mut e) = c.stderr.take() {
                        use std::io::Read;
                        let _ = e.read_to_string(&mut err);
                    }
                    return fail(&mut child, format!("인코더가 멈췄어: {}", err.lines().last().unwrap_or("").trim()));
                }
            }
            None => {
                let p = PathBuf::from(format!("{}_{:05}.png", png_base.display(), i + 1));
                if let Err(e) = rgba.save(&p) {
                    return fail(&mut child, e.to_string());
                }
            }
        }
        pr.done.store(i + 1, Ordering::Relaxed);
    }
    if let Some(mut c) = child {
        drop(c.stdin.take());
        let out = c.wait_with_output().map_err(|e| e.to_string())?;
        cleanup();
        if !out.status.success() {
            return Err(format!("인코딩 실패: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").trim()));
        }
    } else {
        cleanup();
    }
    Ok(format!("영상 내보냄: {} ({w}×{h}, {total}프레임)", job.out.file_name().and_then(|n| n.to_str()).unwrap_or("")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{ShapeKind, ShapeLayer};
    use egui::pos2;

    #[test]
    fn sizes_and_aspect() {
        assert_eq!(size_for(16.0 / 9.0, true, 1920, true), (1920, 1080));
        assert_eq!(size_for(16.0 / 9.0, false, 1080, true), (1920, 1080));
        assert_eq!(size_for(1.0, true, 1081, true), (1082, 1082));
        let r = fit_aspect(Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0)), 2.0);
        assert_eq!((r.width(), r.height(), r.center()), (200.0, 100.0, pos2(50.0, 50.0)));
    }

    /// A moving rectangle exported to H.264 with sound (skipped without ffmpeg).
    #[test]
    fn exports_an_mp4() {
        let Some(t) = crate::video::tools() else { return };
        let mut doc = Document::default();
        let id = doc.insert(
            "s".into(),
            LayerKind::Shape(ShapeLayer { kind: ShapeKind::Rect, a: pos2(0.0, 0.0), b: pos2(40.0, 40.0), color: Color32::RED, width: 0.0, fill: Some(Color32::RED), rotation: 0.0 }),
            None,
        );
        let l = doc.get_mut(id).unwrap();
        let k0 = crate::anim::key_of(l, 0.0);
        let mut k1 = k0;
        k1.t = 1.0;
        k1.center[0] += 100.0;
        l.anim = Some(crate::anim::Anim { keys: vec![k0, k1], ..Default::default() });
        let tone = Arc::new(crate::audio::AudioData::from_samples(48_000, (0..96_000).map(|i| ((i / 2) as f32 * 0.05).sin() * 0.3).collect()));
        let voice = crate::audio::Voice { data: tone, t0: 0.0, src0: 0.0, speed: 1.0, span: (0.0, 10.0), gain: 1.0 };
        let out = std::env::temp_dir().join("dv_export_test.mp4");
        let _ = std::fs::remove_file(&out);
        let job = Job {
            doc,
            region: Region::Fixed(Rect::from_min_max(pos2(-20.0, -20.0), pos2(180.0, 80.0))),
            size: (320, 160),
            fps: 25.0,
            t0: 0.0,
            t1: 1.0,
            codec: Codec::H264,
            quality: 23,
            mbps: None,
            gpu: false,
            bg: Some(Color32::WHITE),
            audio: vec![voice],
            out: out.clone(),
        };
        let p = start(job);
        while p.result.lock().unwrap().is_none() {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let r = p.result.lock().unwrap().take().unwrap();
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(p.done.load(Ordering::Relaxed), 25);
        let info = crate::video::probe(&out).unwrap();
        assert_eq!((info.width, info.height), (320, 160));
        assert!(info.audio);
        assert!((info.duration - 1.0).abs() < 0.1, "{}", info.duration);
        let _ = t;
    }

    /// A video layer in the export: frames come from its own decoder (skipped
    /// without ffmpeg).
    #[test]
    fn exports_a_video_layer_as_png_frames() {
        let Some(t) = crate::video::tools() else { return };
        let dir = std::env::temp_dir().join("dv_vexport_png");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("src.mp4");
        let mut c = Command::new(&t.ffmpeg);
        c.args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=red:size=64x32:rate=10:duration=1,format=yuv420p"]).arg(&clip);
        assert!(c.status().is_ok_and(|s| s.success()));
        let info = crate::video::probe(&clip).unwrap();
        let src = ImageSource { id: next_rev(), rgba: Arc::new(image::RgbaImage::new(64, 32)), encoded: Arc::new(Vec::new()), ext: "png".into(), logical: Some([64, 32]) };
        let mut im = crate::doc::ImageLayer::new(src, egui::pos2(0.0, 0.0));
        im.video = Some(crate::doc::VideoRef { path: clip.to_string_lossy().into(), info, time: 0.0, speed: 1.0, looping: false, mute: false, slot: 1 });
        let mut doc = Document::default();
        doc.insert("v".into(), LayerKind::Image(im), None);
        let job = Job {
            doc,
            region: Region::Fixed(Rect::from_center_size(egui::pos2(0.0, 0.0), vec2(64.0, 32.0))),
            size: (64, 32),
            fps: 10.0,
            t0: 0.0,
            t1: 0.5,
            codec: Codec::Png,
            quality: 0,
            mbps: None,
            gpu: false,
            bg: None,
            audio: Vec::new(),
            out: dir.join("out.png"),
        };
        let p = start(job);
        while p.result.lock().unwrap().is_none() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(p.result.lock().unwrap().take().unwrap().is_ok());
        let f = image::open(dir.join("out_00003.png")).unwrap().to_rgba8();
        let px = f.get_pixel(32, 16);
        assert!(px[0] > 200 && px[1] < 60 && px[3] == 255, "{px:?}");
    }
}
