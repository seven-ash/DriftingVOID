//! Video through an external ffmpeg/ffprobe (docs/VIDEO.md): probing, single-frame
//! decode for scrubbing, and a streaming decoder thread for playback.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "flv", "mpg", "mpeg", "ts", "mts", "m2ts", "3gp"];

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
/// the exe, then PATH.
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

pub const MISSING: &str = "ffmpeg 없음 · winget install Gyan.FFmpeg 또는 %APPDATA%\\DriftingVOID\\ffmpeg";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoInfo {
    /// Displayed size (after the rotation tag).
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub audio: bool,
    pub codec: String,
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
    let t = tools().ok_or(MISSING)?;
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
        .ok_or("영상 트랙 없음")?;
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
    })
}

fn decode_args(path: &Path, t: f64, size: (u32, u32)) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> = Vec::new();
    for s in ["-v", "error", "-nostdin", "-ss"] {
        a.push(s.into());
    }
    a.push(format!("{:.4}", t.max(0.0)).into());
    a.push("-i".into());
    a.push(path.into());
    a.push("-vf".into());
    a.push(format!("scale={}:{}:flags=bicubic", size.0, size.1).into());
    for s in ["-an", "-sn", "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"] {
        a.push(s.into());
    }
    a
}

/// One frame at `t` seconds, `size` pixels.
pub fn frame_at(path: &Path, t: f64, size: (u32, u32)) -> Result<image::RgbaImage, String> {
    let tl = tools().ok_or(MISSING)?;
    let mut args = decode_args(path, t, size);
    // a single frame: insert "-frames:v 1" before the output options
    let at = args.len() - 7;
    args.insert(at, "1".into());
    args.insert(at, "-frames:v".into());
    let out = no_window(Command::new(&tl.ffmpeg).args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()))
        .output()
        .map_err(|e| e.to_string())?;
    let need = (size.0 * size.1 * 4) as usize;
    if out.stdout.len() < need {
        // past the end: step back a little
        if t > 0.05 && out.stdout.is_empty() {
            return frame_at(path, (t - 0.25).max(0.0), size);
        }
        return Err(String::from_utf8_lossy(&out.stderr).trim().lines().last().unwrap_or("프레임 읽기 실패").to_string());
    }
    let mut px = out.stdout;
    px.truncate(need);
    image::RgbaImage::from_raw(size.0, size.1, px).ok_or_else(|| "프레임 크기 불일치".into())
}

/// Streaming playback decoder: frames from `t0` on, at `size`, through a small
/// bounded channel. Dropping it stops ffmpeg.
pub struct Player {
    pub rx: Receiver<(f64, image::RgbaImage)>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    /// The stream ended (end of file or an error).
    pub ended: Arc<AtomicBool>,
}

impl Player {
    pub fn start(path: &Path, t0: f64, fps: f64, size: (u32, u32)) -> Result<Self, String> {
        let tl = tools().ok_or(MISSING)?;
        let mut child = no_window(Command::new(&tl.ffmpeg).args(decode_args(path, t0, size)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()))
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut out = child.stdout.take().ok_or("ffmpeg 출력 없음")?;
        let (tx, rx) = sync_channel(3);
        let stop = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(Some(child)));
        let (st, en) = (stop.clone(), ended.clone());
        let n = (size.0 * size.1 * 4) as usize;
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
                let Some(img) = image::RgbaImage::from_raw(size.0, size.1, buf) else { break };
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

impl Drop for Player {
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

    #[test]
    fn timecodes() {
        assert_eq!(timecode(75.5), "01:15.50");
        assert_eq!(timecode(3725.0), "1:02:05.00");
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
