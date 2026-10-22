//! Frosted-glass settings (on/off, strength, rim, shadow) and the card shape.
//! The cards themselves are drawn by `glass_gpu` from the canvas picture.

use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Rect, Shape};

static ENABLED: AtomicBool = AtomicBool::new(true);
static SHADOW_ON: AtomicBool = AtomicBool::new(false);
static PALETTE_NEAR: AtomicBool = AtomicBool::new(false);

/// Palette option: show the "similar colours" map (off by default).
pub fn palette_near() -> bool {
    PALETTE_NEAR.load(Ordering::Relaxed)
}

pub fn set_palette_near(on: bool) {
    PALETTE_NEAR.store(on, Ordering::Relaxed);
}

pub fn shadow() -> bool {
    SHADOW_ON.load(Ordering::Relaxed)
}

pub fn set_shadow(on: bool) {
    SHADOW_ON.store(on, Ordering::Relaxed);
}

static WGPU: AtomicBool = AtomicBool::new(false);
/// The pass in which the GPU canvas last gave the glass a picture.
static SCENE_PASS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// `glass_gpu` has its resources (set once at start).
pub fn set_ready(on: bool) {
    WGPU.store(on, Ordering::Relaxed);
}

/// The GPU canvas drew this pass (glass has something to show).
pub fn scene_drawn(pass: u64) {
    SCENE_PASS.store(pass, Ordering::Relaxed);
}

static CUR_PASS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A new pass: glass needs the canvas drawn this pass or the last
/// (not in the 3D workspace, where cards are opaque).
pub fn begin_pass(pass: u64) {
    CUR_PASS.store(pass, Ordering::Relaxed);
}

/// Glass is on and has a canvas picture from this pass or the last.
pub fn available() -> bool {
    let (drawn, now) = (SCENE_PASS.load(Ordering::Relaxed), CUR_PASS.load(Ordering::Relaxed));
    WGPU.load(Ordering::Relaxed) && ENABLED.load(Ordering::Relaxed) && drawn != u64::MAX && drawn + 1 >= now
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Persisted look settings (%APPDATA%/DriftingVOID/settings.json).
fn settings_path() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|a| std::path::Path::new(&a).join("DriftingVOID").join("settings.json"))
}

pub fn load_settings() {
    let Some(p) = settings_path() else { return };
    let Ok(text) = std::fs::read_to_string(p) else { return };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    if let Some(on) = v.get("glass").and_then(|x| x.as_bool()) {
        set_enabled(on);
    }
    if let Some(b) = v.get("glass_band").and_then(|x| x.as_f64()) {
        set_band(b as f32);
    }
    if let Some(on) = v.get("palette_near").and_then(|x| x.as_bool()) {
        set_palette_near(on);
    }
    if let Some(on) = v.get("shadow").and_then(|x| x.as_bool()) {
        set_shadow(on);
    }
    if let Some(k) = v.get("glass_strength").and_then(|x| x.as_f64()) {
        set_strength(k as f32);
    }
}

pub fn save_settings() {
    // debug runs (DV_* flags) never touch the saved settings
    if std::env::vars_os().any(|(k, _)| k.to_string_lossy().starts_with("DV_")) {
        return;
    }
    let Some(p) = settings_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let v = serde_json::json!({ "glass": enabled(), "glass_strength": strength(), "shadow": shadow(), "glass_band": band(), "palette_near": palette_near() });
    let _ = std::fs::write(p, serde_json::to_string_pretty(&v).unwrap_or_default());
}

#[derive(Clone, Copy)]
pub struct Look {
    pub radius: f32,
    /// Max sample offset at the rim, points.
    pub refract: f32,
    /// Tint colour (straight RGBA 0..1) laid over the blur.
    pub tint: [f32; 4],
    /// Red/blue split at the rim, points.
    pub dispersion: f32,
}

static STRENGTH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x4030_0000); // 2.75

static BAND: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x40E0_0000); // 7.0

/// Width of the refracting rim in points.
pub fn band() -> f32 {
    f32::from_bits(BAND.load(Ordering::Relaxed))
}

pub fn set_band(v: f32) {
    BAND.store(v.clamp(1.0, 24.0).to_bits(), Ordering::Relaxed);
}

/// Global multiplier for refraction + dispersion (0 = flat frosted glass).
pub fn strength() -> f32 {
    f32::from_bits(STRENGTH.load(Ordering::Relaxed))
}

pub fn set_strength(v: f32) {
    STRENGTH.store(v.clamp(0.0, 4.0).to_bits(), Ordering::Relaxed);
}

pub fn shape(card: Rect, look: Look) -> Shape {
    crate::glass_gpu::shape(card, look)
}
