// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.

//! Video through an external ffmpeg/ffprobe (notes/VIDEO.md): probing, single-frame
//! decode for scrubbing, and a streaming decoder thread for playback.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "flv", "mpg", "mpeg", "ts", "mts", "m2ts", "3gp"];

/// Animated GIFs come in as videos (a single-frame GIF stays a photo).
pub fn is_gif(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

/// Longest side of preview frames (playback and scrubbing).
pub const PREVIEW_SIDE: u32 = 1920;

pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn works(exe: &Path) -> bool {
    no_window(Command::new(exe).arg("-version").stdout(Stdio::null()).stderr(Stdio::null())).status().is_ok_and(|s| s.success())
}

/// ffmpeg and ffprobe, looked for once: %APPDATA%\DriftingVOID\ffmpeg(\bin), next to
/// the exe or in its ffmpeg\bin (where the beta's setup puts it), then PATH.
pub fn tools() -> Option<&'static Tools> {
    static T: OnceLock<Option<Tools>> = OnceLock::new();
    T.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(a) = std::env::var_os("APPDATA") {
            let base = Path::new(&a).join("DriftingVOID").join("ffmpeg");
            dirs.push(base.join("bin"));
            dirs.push(base);
        }
        if let Some(d) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
            dirs.push(d.join("ffmpeg").join("bin"));
            dirs.push(d);
        }
        for d in dirs {
            let (f, p) = (d.join("ffmpeg.exe"), d.join("ffprobe.exe"));
            if f.exists() && p.exists() && works(&f) {
                return Some(Tools { ffmpeg: f, ffprobe: p });
            }
        }
        let (f, p) = (PathBuf::from("ffmpeg"), PathBuf::from("ffprobe"));
        (works(&f) && works(&p)).then_some(Tools { ffmpeg: f, ffprobe: p })
    })
    .as_ref()
}

/// An HEIF still decoded by ffmpeg (grids of tiles and the rotation tag are
/// applied by ffmpeg 7.1+), as RGBA. ffmpeg needs a seekable file, so the
/// bytes go through a temporary one. An alpha plane, rare in photos, is not
/// read.
pub fn decode_heif(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let t = tools().ok_or_else(missing)?;
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("dv_heif_{}_{n}.heic", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    let out = no_window(Command::new(&t.ffmpeg).args(["-hide_banner", "-loglevel", "error", "-i"]).arg(&tmp).args(["-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "-pix_fmt", "rgba", "-"]))
        .stdin(Stdio::null())
        .output();
    let _ = std::fs::remove_file(&tmp);
    let out = out.map_err(|e| e.to_string())?;
    if !out.status.success() || out.stdout.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(crate::i18n::trf("HEIC 읽기 실패: {}", &[format!("{}", err.lines().next().unwrap_or("ffmpeg"))]));
    }
    image::load_from_memory_with_format(&out.stdout, image::ImageFormat::Png).map(|i| i.to_rgba8()).map_err(|e| e.to_string())
}

/// What to say when ffmpeg is not found.
pub fn missing() -> &'static str {
    crate::i18n::tr("ffmpeg 없음 · 설치 프로그램 다시 실행 또는 winget install Gyan.FFmpeg")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoInfo {
    /// Displayed size (after the rotation tag).
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub audio: bool,
    pub codec: String,
    /// The frames carry transparency (an animated GIF, a video with an alpha
    /// channel): decoded as RGBA everywhere, never NV12.
    #[serde(default)]
    pub alpha: bool,
}

impl VideoInfo {
    pub fn frame_len(&self) -> f64 {
        1.0 / self.fps.max(1.0)
    }

    /// Preview size: the displayed size scaled to fit PREVIEW_SIDE, even dimensions.
    pub fn preview_size(&self, side: u32) -> (u32, u32) {
        let (w, h) = (self.width.max(2), self.height.max(2));
        let k = (side as f64 / w.max(h) as f64).min(1.0);
        let even = |x: f64| (((x / 2.0).round() as u32) * 2).max(2);
        (even(w as f64 * k), even(h as f64 * k))
    }
}

fn ratio(s: &str) -> Option<f64> {
    match s.split_once('/') {
        Some((a, b)) => {
            let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
            (b != 0.0).then(|| a / b)
        }
        None => s.parse().ok(),
    }
}

pub fn probe(path: &Path) -> Result<VideoInfo, String> {
    let t = tools().ok_or_else(missing)?;
    let out = no_window(
        Command::new(&t.ffprobe)
            .args(["-v", "error", "-print_format", "json", "-show_streams", "-show_format"])
            .arg(path)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped()),
    )
    .output()
    .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    parse_probe(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_probe(json: &str) -> Result<VideoInfo, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let vs = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64() != Some(1))
        .ok_or(crate::i18n::tr("영상 트랙 없음"))?;
    let (mut w, mut h) = (vs["width"].as_u64().unwrap_or(0) as u32, vs["height"].as_u64().unwrap_or(0) as u32);
    // rotation: side data (newer) or the "rotate" tag (older files)
    let rot = vs["side_data_list"]
        .as_array()
        .and_then(|l| l.iter().find_map(|d| d["rotation"].as_f64()))
        .or_else(|| vs["tags"]["rotate"].as_str().and_then(|r| r.parse().ok()))
        .unwrap_or(0.0);
    if (rot.abs() as i64) % 180 == 90 {
        std::mem::swap(&mut w, &mut h);
    }
    let fps = vs["avg_frame_rate"].as_str().and_then(ratio).filter(|f| *f > 0.0).or_else(|| vs["r_frame_rate"].as_str().and_then(ratio)).unwrap_or(30.0);
    let duration = vs["duration"]
        .as_str()
        .and_then(|d| d.parse().ok())
        .or_else(|| v["format"]["duration"].as_str().and_then(|d| d.parse().ok()))
        .unwrap_or(0.0);
    Ok(VideoInfo {
        width: w,
        height: h,
        fps,
        duration,
        audio: streams.iter().any(|s| s["codec_type"] == "audio"),
        codec: vs["codec_name"].as_str().unwrap_or("?").to_string(),
        alpha: {
            let pf = vs["pix_fmt"].as_str().unwrap_or("");
            vs["codec_name"] == "gif" || pf.starts_with("yuva") || pf.starts_with("ya") || ["rgba", "bgra", "argb", "abgr", "gbrap", "pal8"].iter().any(|a| pf.starts_with(a))
        },
    })
}

fn decode_args(path: &Path, t: f64, size: (u32, u32), fps: Option<f64>) -> Vec<std::ffi::OsString> {
    decode_args_fmt(path, t, size, false, "auto", fps)
}

/// `nv12`: NV12 frames in BT.709 limited range, whatever the source's colours
/// (the export's video frames; the GPU turns them into RGB).
fn decode_args_fmt(path: &Path, t: f64, size: (u32, u32), nv12: bool, hwaccel: &str, fps: Option<f64>) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> = Vec::new();
    // DV_DECODE_HW=0: software decoding; =name: that hwaccel
    let hw = std::env::var("DV_DECODE_HW").unwrap_or_else(|_| hwaccel.into());
    for s in ["-v", "error", "-nostdin"] {
        a.push(s.into());
    }
    if hw != "0" {
        a.push("-hwaccel".into());
        a.push(hw.into());
    }
    a.push("-ss".into());
    a.push(format!("{:.4}", t.max(0.0)).into());
    a.push("-i".into());
    a.push(path.into());
    a.push("-vf".into());
    // a GIF's frames last as long as each says: made steady at its average
    // rate, as the players count frames by it
    let steady = match fps {
        Some(f) if is_gif(path) => format!("fps={f:.6},"),
        _ => String::new(),
    };
    if nv12 {
        a.push(format!("{steady}scale={}:{}:flags=bicubic:out_color_matrix=bt709:out_range=tv", size.0, size.1).into());
    } else {
        a.push(format!("{steady}scale={}:{}:flags=bicubic", size.0, size.1).into());
    }
    for s in ["-an", "-sn", "-f", "rawvideo", "-pix_fmt", if nv12 { "nv12" } else { "rgba" }, "pipe:1"] {
        a.push(s.into());
    }
    a
}

/// One frame at `t` seconds, `size` pixels.
pub fn frame_at(path: &Path, t: f64, size: (u32, u32)) -> Result<image::RgbaImage, String> {
    // a file that is not there has no last frame to step back to (before
    // 0.104.5 this ran ffmpeg twice for every quarter second of `t`)
    if !path.is_file() {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().to_string());
        return Err(crate::i18n::trf("영상 파일을 읽을 수 없음: {}", &[name]));
    }
    frame_at_tries(path, t, size, 2)
}

/// `frame_at`, stepping back at most `back` times when nothing comes at `t`.
fn frame_at_tries(path: &Path, t: f64, size: (u32, u32), back: u8) -> Result<image::RgbaImage, String> {
    let tl = tools().ok_or_else(missing)?;
    let mut args = decode_args(path, t, size, None);
    // a single frame: insert "-frames:v 1" before the output options
    let at = args.len() - 7;
    args.insert(at, "1".into());
    args.insert(at, "-frames:v".into());
    let out = no_window(Command::new(&tl.ffmpeg).args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()))
        .output()
        .map_err(|e| e.to_string())?;
    let need = (size.0 * size.1 * 4) as usize;
    if out.stdout.len() < need {
        // at or past the last frame (a seek there finds nothing): the last frame
        // itself, from decoding the final second (stepping back a fixed time
        // would land several frames early)
        if t > 0.05 && out.stdout.is_empty() && back > 0 {
            if let Some(img) = last_frame(path, size) {
                return Ok(img);
            }
            return frame_at_tries(path, (t - 0.25).max(0.0), size, back - 1);
        }
        return Err(String::from_utf8_lossy(&out.stderr).trim().lines().last().unwrap_or(crate::i18n::tr("프레임 읽기 실패")).to_string());
    }
    let mut px = out.stdout;
    px.truncate(need);
    image::RgbaImage::from_raw(size.0, size.1, px).ok_or_else(|| crate::i18n::tr("프레임 크기 불일치").into())
}

/// The video's very last frame, at `size`.
fn last_frame(path: &Path, size: (u32, u32)) -> Option<image::RgbaImage> {
    let tl = tools()?;
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    for s in ["-v", "error", "-nostdin", "-hwaccel", "auto", "-sseof", "-1", "-i"] {
        args.push(s.into());
    }
    args.push(path.into());
    args.push("-vf".into());
    args.push(format!("scale={}:{}:flags=bicubic", size.0, size.1).into());
    for s in ["-an", "-sn", "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"] {
        args.push(s.into());
    }
    let out = no_window(Command::new(&tl.ffmpeg).args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())).output().ok()?;
    let need = (size.0 * size.1 * 4) as usize;
    let n = out.stdout.len() / need;
    if n == 0 {
        return None;
    }
    image::RgbaImage::from_raw(size.0, size.1, out.stdout[(n - 1) * need..n * need].to_vec())
}

/// Streaming playback decoder: frames from `t0` on, at `size`, through a small
/// bounded channel. Dropping it stops ffmpeg.
pub struct Player<T = image::RgbaImage> {
    pub rx: Receiver<(f64, T)>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    /// The stream ended (end of file or an error).
    pub ended: Arc<AtomicBool>,
}

impl Player {
    pub fn start(path: &Path, t0: f64, fps: f64, size: (u32, u32)) -> Result<Self, String> {
        Player::spawn(decode_args(path, t0, size, Some(fps)), (size.0 * size.1 * 4) as usize, t0, fps, move |buf| image::RgbaImage::from_raw(size.0, size.1, buf))
    }
}

impl Player<Vec<u8>> {
    /// As `start`, with NV12 frames (BT.709 limited range; even sizes only),
    /// decoded with `hwaccel` (ffmpeg's name: "cuda" on NVIDIA, else "auto").
    pub fn start_nv12(path: &Path, t0: f64, fps: f64, size: (u32, u32), hwaccel: &str) -> Result<Self, String> {
        Player::spawn(decode_args_fmt(path, t0, size, true, hwaccel, Some(fps)), (size.0 * size.1 * 3 / 2) as usize, t0, fps, Some)
    }

    /// As `start`, RGBA frames as raw bytes (a transparent video for the
    /// video cache), decoded with `hwaccel`.
    pub fn start_rgba(path: &Path, t0: f64, fps: f64, size: (u32, u32), hwaccel: &str) -> Result<Self, String> {
        Player::spawn(decode_args_fmt(path, t0, size, false, hwaccel, Some(fps)), (size.0 * size.1 * 4) as usize, t0, fps, Some)
    }
}

impl<T: Send + 'static> Player<T> {
    fn spawn(args: Vec<std::ffi::OsString>, n: usize, t0: f64, fps: f64, wrap: impl Fn(Vec<u8>) -> Option<T> + Send + 'static) -> Result<Self, String> {
        let tl = tools().ok_or_else(missing)?;
        let mut child = no_window(Command::new(&tl.ffmpeg).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()))
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut out = child.stdout.take().ok_or(crate::i18n::tr("ffmpeg 출력 없음"))?;
        // a few frames ahead: a pre-roll fills it, and a late frame doesn't stall
        let (tx, rx) = sync_channel(6);
        let stop = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(Some(child)));
        let (st, en) = (stop.clone(), ended.clone());
        let dt = 1.0 / fps.max(1.0);
        std::thread::spawn(move || {
            let mut i = 0u64;
            loop {
                if st.load(Ordering::Relaxed) {
                    break;
                }
                let mut buf = vec![0u8; n];
                if out.read_exact(&mut buf).is_err() {
                    break;
                }
                let Some(img) = wrap(buf) else { break };
                if tx.send((t0 + i as f64 * dt, img)).is_err() {
                    break;
                }
                i += 1;
            }
            en.store(true, Ordering::Relaxed);
        });
        Ok(Self { rx, stop, child, ended })
    }
}

impl<T> Drop for Player<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut c) = self.child.lock() {
            if let Some(mut c) = c.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

/// "mm:ss.cc" (or "h:mm:ss.cc").
pub fn timecode(t: f64) -> String {
    let t = t.max(0.0);
    let h = (t / 3600.0) as u64;
    let m = ((t % 3600.0) / 60.0) as u64;
    let s = t % 60.0;
    if h > 0 { format!("{h}:{m:02}:{s:05.2}") } else { format!("{m:02}:{s:05.2}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of a file that is not there fails at once, however late the
    /// time (before 0.104.5: two ffmpeg runs for every quarter second of it).
    #[test]
    fn a_missing_file_fails_at_once() {
        let t = std::time::Instant::now();
        assert!(frame_at(Path::new("Z:/no/such/clip.mp4"), 600.0, (64, 48)).is_err());
        assert!(t.elapsed().as_secs_f64() < 0.5, "{:?}", t.elapsed());
    }

    #[test]
    fn probe_json_with_rotation() {
        let j = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,
            "avg_frame_rate":"30000/1001","r_frame_rate":"30000/1001","duration":"12.5",
            "side_data_list":[{"rotation":-90}]},{"codec_type":"audio"}],"format":{"duration":"12.6"}}"#;
        let i = parse_probe(j).unwrap();
        assert_eq!((i.width, i.height), (1080, 1920));
        assert!((i.fps - 29.97).abs() < 0.01);
        assert!((i.duration - 12.5).abs() < 1e-9);
        assert!(i.audio);
        assert_eq!(i.preview_size(1920), (1080, 1920));
        assert_eq!(i.preview_size(960), (540, 960));
    }

    /// A phone photo: HEIC, 1200×800 tagged to turn 90° (orientation 6),
    /// comes in upright, 800×1200, with its corner mark at the top right
    /// (skipped without ffmpeg).
    #[test]
    fn heic_upright() {
        if tools().is_none() {
            return;
        }
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/rot6.heic")).unwrap();
        assert!(crate::io::is_heif(&bytes));
        let src = crate::io::decode_image(bytes, None).unwrap();
        assert_eq!(src.ext, "heic");
        assert_eq!(src.rgba.dimensions(), (800, 1200));
        // the red 300×200 block at the original top left turns to the top right
        let p = src.rgba.get_pixel(700, 100);
        assert!(p[0] > 180 && p[1] < 120, "{p:?}");
        let q = src.rgba.get_pixel(100, 100);
        assert!(q[0] > 150 && q[1] > 150, "{q:?}");
    }

    #[test]
    fn timecodes() {
        assert_eq!(timecode(75.5), "01:15.50");
        assert_eq!(timecode(3725.0), "1:02:05.00");
    }

    /// Past the last frame, a still is the last frame itself, not one a few
    /// frames earlier (skipped without ffmpeg).
    #[test]
    fn past_the_end_is_the_last_frame() {
        let Some(t) = tools() else { return };
        let dir = std::env::temp_dir().join("dv_video_end");
        let _ = std::fs::create_dir_all(&dir);
        let clip = dir.join("count.mp4");
        // 10 frames at 10 fps, each a different hue (36° apart)
        let ok = Command::new(&t.ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=red:size=64x48:rate=10:duration=1,hue=h=t*360", "-pix_fmt", "yuv420p", "-c:v", "libx264", "-crf", "10"])
            .arg(&clip)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok);
        // mean difference per channel (decoders may differ by a level or two)
        let diff = |a: &image::RgbaImage, b: &image::RgbaImage| a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as f64).sum::<f64>() / a.as_raw().len() as f64;
        let last = frame_at(&clip, 0.89, (64, 48)).unwrap();
        let before = frame_at(&clip, 0.79, (64, 48)).unwrap();
        assert!(diff(&last, &before) > 5.0, "frames differ: {}", diff(&last, &before));
        for t in [0.95, 1.0, 1.5] {
            let d = diff(&frame_at(&clip, t, (64, 48)).unwrap(), &last);
            assert!(d < 1.5, "at {t}: mean difference {d} from the last frame");
        }
    }

    /// Real decode through ffmpeg (skipped when ffmpeg is not installed).
    #[test]
    fn decode_a_generated_clip() {
        let Some(t) = tools() else { return };
        let dir = std::env::temp_dir().join("dv_video_test");
        let _ = std::fs::create_dir_all(&dir);
        let clip = dir.join("clip.mp4");
        let ok = no_window(Command::new(&t.ffmpeg).args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=2", "-pix_fmt", "yuv420p"]).arg(&clip))
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok);
        let info = probe(&clip).unwrap();
        assert_eq!((info.width, info.height), (320, 180));
        assert!((info.fps - 25.0).abs() < 0.01);
        let f = frame_at(&clip, 1.0, (160, 90)).unwrap();
        assert_eq!(f.dimensions(), (160, 90));
        let p = Player::start(&clip, 0.5, info.fps, (160, 90)).unwrap();
        let (t0, img) = p.rx.recv().unwrap();
        assert!((t0 - 0.5).abs() < 1e-9);
        assert_eq!(img.dimensions(), (160, 90));
        let n = p.rx.iter().count();
        assert!(n > 20, "{n} frames");
    }
}

#[cfg(test)]
mod bench {
    /// Time to the first frame and to the tenth of a streaming decoder
    /// (DV_BENCH_VIDEO=path). `cargo test --release decoder_start -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn decoder_start() {
        let Some(p) = std::env::var_os("DV_BENCH_VIDEO") else { return };
        let p = std::path::PathBuf::from(p);
        let info = super::probe(&p).unwrap();
        for t0 in [0.0, info.duration * 0.5] {
            let t = std::time::Instant::now();
            let pl = super::Player::start(&p, t0, info.fps, info.preview_size(super::PREVIEW_SIDE)).unwrap();
            let _ = pl.rx.recv().unwrap();
            let first = t.elapsed().as_secs_f64() * 1e3;
            for _ in 0..9 {
                let _ = pl.rx.recv().unwrap();
            }
            eprintln!("{} at {t0:.1}s ({}x{} @ {:.1} fps): first frame {first:.0} ms, ten frames {:.0} ms", p.display(), info.width, info.height, info.fps, t.elapsed().as_secs_f64() * 1e3);
        }
    }
}
