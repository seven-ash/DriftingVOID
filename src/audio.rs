// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.

//! Audio (notes/VIDEO.md, V3): files and video soundtracks decoded to PCM through
//! ffmpeg, and a small mixer on the default output device whose position is the
//! timeline clock while it plays.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub const AUDIO_EXTS: &[&str] = &["mp3", "wav", "flac", "ogg", "m4a", "aac", "opus", "wma", "aiff", "aif"];

/// Waveform resolution: one peak per this many seconds.
pub const PEAK_SEC: f64 = 0.02;

/// Decoded stereo audio at the output's sample rate.
pub struct AudioData {
    pub rate: u32,
    /// Interleaved L, R.
    pub samples: Vec<f32>,
    /// Max |sample| per PEAK_SEC window, 0..1.
    pub peaks: Vec<f32>,
}

impl AudioData {
    pub fn duration(&self) -> f64 {
        self.samples.len() as f64 / 2.0 / self.rate as f64
    }

    /// Linearly interpolated stereo sample at `s` seconds (None outside).
    fn at(&self, s: f64) -> Option<(f32, f32)> {
        if s < 0.0 {
            return None;
        }
        let f = s * self.rate as f64;
        let i = f as usize;
        let n = self.samples.len() / 2;
        if i + 1 >= n {
            return None;
        }
        let u = (f - i as f64) as f32;
        let (a, b) = (&self.samples[2 * i..2 * i + 2], &self.samples[2 * i + 2..2 * i + 4]);
        Some((a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u))
    }

    pub fn from_samples(rate: u32, samples: Vec<f32>) -> Self {
        let win = ((rate as f64 * PEAK_SEC) as usize).max(1) * 2;
        let peaks = samples.chunks(win).map(|c| c.iter().fold(0.0f32, |m, x| m.max(x.abs())).min(1.0)).collect();
        Self { rate, samples, peaks }
    }
}

/// Decodes the audio of a file (an audio file or a video's soundtrack).
pub fn decode(path: &Path, rate: u32) -> Result<AudioData, String> {
    let t = crate::video::tools().ok_or_else(crate::video::missing)?;
    let mut cmd = Command::new(&t.ffmpeg);
    cmd.args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-vn", "-sn", "-ac", "2", "-ar", &rate.to_string(), "-f", "f32le", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.stdout.len() < 8 {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().last().unwrap_or(crate::i18n::tr("소리 트랙 없음")).to_string());
    }
    let samples: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    Ok(AudioData::from_samples(rate, samples))
}

/// One sound placed on the timeline: source time `src0` plays at timeline time `t0`,
/// `speed` seconds of source per second, heard only inside `span`.
#[derive(Clone)]
pub struct Voice {
    pub data: Arc<AudioData>,
    pub t0: f64,
    pub src0: f64,
    pub speed: f64,
    pub span: (f64, f64),
    pub gain: f32,
    /// Keyed volume over timeline time (replaces `gain` where present).
    pub env: Option<Arc<Envelope>>,
    /// A looping clip: past this many seconds the source starts over.
    pub wrap: Option<f64>,
}

/// Volume sampled every `dt` seconds of timeline time from `t0`.
#[derive(Debug)]
pub struct Envelope {
    pub t0: f64,
    pub dt: f64,
    pub v: Vec<f32>,
}

impl Envelope {
    pub fn at(&self, t: f64) -> f32 {
        let x = ((t - self.t0) / self.dt).max(0.0);
        let i = x.floor() as usize;
        match (self.v.get(i), self.v.get(i + 1)) {
            (Some(a), Some(b)) => a + (b - a) * (x - i as f64) as f32,
            (Some(a), None) => *a,
            _ => self.v.last().copied().unwrap_or(1.0),
        }
    }
}

/// Mixed stereo sample at timeline time `t`.
pub fn mix_at(voices: &[Voice], t: f64) -> (f32, f32) {
    let (mut l, mut r) = (0.0f32, 0.0f32);
    for v in voices {
        if t >= v.span.0 && t < v.span.1 {
            let mut s = v.src0 + (t - v.t0) * v.speed;
            if let Some(d) = v.wrap.filter(|d| *d > 0.0 && s >= *d) {
                s = s.rem_euclid(d);
            }
            if let Some((a, b)) = v.data.at(s) {
                let g = v.env.as_ref().map_or(v.gain, |e| e.at(t));
                l += a * g;
                r += b * g;
            }
        }
    }
    (l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0))
}

struct Mix {
    voices: Vec<Voice>,
    playing: bool,
    pos: f64,
    /// When the device last asked for samples (or play began): a device that
    /// has gone stops asking, and its position stops with it.
    fed: std::time::Instant,
    /// The device reported an error (unplugged, the default changed).
    broken: bool,
}

/// Without samples asked for this long while playing, the device counts as gone.
const STALL: std::time::Duration = std::time::Duration::from_millis(600);

/// The default output device, playing a set of voices from a position.
pub struct Output {
    _stream: cpal::Stream,
    mix: Arc<Mutex<Mix>>,
    pub rate: u32,
}

impl Output {
    pub fn open() -> Result<Self, String> {
        let host = cpal::default_host();
        let dev = host.default_output_device().ok_or(crate::i18n::tr("소리 출력 장치 없음"))?;
        let sup = dev.default_output_config().map_err(|e| e.to_string())?;
        let rate = sup.sample_rate();
        let ch = sup.channels() as usize;
        let mix = Arc::new(Mutex::new(Mix { voices: Vec::new(), playing: false, pos: 0.0, fed: std::time::Instant::now(), broken: false }));
        let config = sup.config();
        let stream = match sup.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&dev, config, ch, rate, mix.clone()),
            cpal::SampleFormat::I16 => build::<i16>(&dev, config, ch, rate, mix.clone()),
            cpal::SampleFormat::U16 => build::<u16>(&dev, config, ch, rate, mix.clone()),
            cpal::SampleFormat::I32 => build::<i32>(&dev, config, ch, rate, mix.clone()),
            f => return Err(crate::i18n::trf("지원하지 않는 소리 형식 {}", &[format!("{f:?}")])),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { _stream: stream, mix, rate })
    }

    pub fn play(&self, voices: Vec<Voice>, pos: f64) {
        if let Ok(mut m) = self.mix.lock() {
            m.voices = voices;
            m.pos = pos;
            m.playing = true;
            m.fed = std::time::Instant::now();
        }
    }

    /// The device is gone (it reported an error, or stopped asking for
    /// samples while playing): this output is to be dropped and opened again.
    pub fn broken(&self) -> bool {
        self.mix.lock().map_or(true, |m| m.broken || (m.playing && !m.voices.is_empty() && m.fed.elapsed() > STALL))
    }

    pub fn stop(&self) {
        if let Ok(mut m) = self.mix.lock() {
            m.playing = false;
            m.voices.clear();
        }
    }

    /// Timeline position while playing (None once the device is gone: the
    /// caller's own clock carries on).
    pub fn pos(&self) -> Option<f64> {
        self.mix.lock().ok().filter(|m| m.playing && !m.voices.is_empty() && !m.broken && m.fed.elapsed() <= STALL).map(|m| m.pos)
    }
}

fn build<T>(dev: &cpal::Device, config: cpal::StreamConfig, ch: usize, rate: u32, mix: Arc<Mutex<Mix>>) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let dt = 1.0 / rate as f64;
    let on_err = mix.clone();
    dev.build_output_stream(
        config,
        move |out: &mut [T], _| {
            let Ok(mut m) = mix.lock() else {
                out.iter_mut().for_each(|s| *s = T::from_sample(0.0));
                return;
            };
            m.fed = std::time::Instant::now();
            for frame in out.chunks_mut(ch.max(1)) {
                let (l, r) = if m.playing {
                    let t = m.pos;
                    let s = mix_at(&m.voices, t);
                    m.pos = t + dt;
                    s
                } else {
                    (0.0, 0.0)
                };
                if frame.len() == 1 {
                    frame[0] = T::from_sample((l + r) * 0.5);
                } else {
                    frame[0] = T::from_sample(l);
                    frame[1] = T::from_sample(r);
                    for x in frame.iter_mut().skip(2) {
                        *x = T::from_sample(0.0);
                    }
                }
            }
        },
        move |e| {
            eprintln!("audio: {e}");
            if let Ok(mut m) = on_err.lock() {
                m.broken = true;
            }
        },
        None,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(rate: u32, secs: f64) -> Arc<AudioData> {
        let n = (rate as f64 * secs) as usize;
        let mut s = Vec::with_capacity(n * 2);
        for i in 0..n {
            let v = i as f32 / n as f32;
            s.push(v);
            s.push(-v);
        }
        Arc::new(AudioData::from_samples(rate, s))
    }

    #[test]
    fn mixing_follows_the_mapping() {
        let d = ramp(1000, 2.0);
        let v = Voice { data: d.clone(), t0: 5.0, src0: 0.5, speed: 2.0, span: (5.0, 6.0), gain: 1.0, env: None, wrap: None };
        // a looping clip of 0.75 s: 1.0 s in sounds as 0.25 s in
        let w = Voice { data: d.clone(), t0: 0.0, src0: 0.0, speed: 1.0, span: (0.0, 9.0), gain: 1.0, env: None, wrap: Some(0.75) };
        let once = Voice { wrap: None, ..w.clone() };
        assert_eq!(mix_at(&[w.clone()], 1.0), mix_at(&[once.clone()], 0.25));
        assert_eq!(mix_at(&[w], 0.5), mix_at(&[once], 0.5));
        // timeline 5.25 -> source 1.0 -> half of the ramp
        let (l, r) = mix_at(std::slice::from_ref(&v), 5.25);
        assert!((l - 0.5).abs() < 1e-2 && (r + 0.5).abs() < 1e-2, "{l} {r}");
        assert_eq!(mix_at(std::slice::from_ref(&v), 4.9), (0.0, 0.0));
        assert_eq!(mix_at(std::slice::from_ref(&v), 6.0), (0.0, 0.0));
        assert!((d.duration() - 2.0).abs() < 1e-9);
        assert_eq!(d.peaks.len(), 100);
    }

    #[test]
    fn decode_a_generated_tone() {
        let Some(t) = crate::video::tools() else { return };
        let path = std::env::temp_dir().join("dv_tone.wav");
        let mut c = Command::new(&t.ffmpeg);
        c.args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1"]).arg(&path);
        assert!(c.status().is_ok_and(|s| s.success()));
        let a = decode(&path, 48000).unwrap();
        assert!((a.duration() - 1.0).abs() < 0.01, "{}", a.duration());
        assert!(a.peaks.iter().cloned().fold(0.0, f32::max) > 0.05);
    }

    /// The real device: playing moves the clock (skipped without a sound card).
    #[test]
    fn output_clock_runs() {
        let Ok(out) = Output::open() else { return };
        let d = ramp(out.rate, 2.0);
        out.play(vec![Voice { data: d, t0: 0.0, src0: 0.0, speed: 1.0, span: (0.0, 2.0), gain: 0.0, env: None, wrap: None }], 1.0);
        std::thread::sleep(std::time::Duration::from_millis(400));
        let p = out.pos().unwrap();
        assert!(p > 1.1 && p < 1.8, "{p}");
        out.stop();
        assert!(out.pos().is_none());
    }
}
