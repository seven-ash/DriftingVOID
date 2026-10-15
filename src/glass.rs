//! Frosted-glass panels: a GL paint callback that copies the pixels under a
//! card, blurs them, and composites them back with edge refraction, a little
//! chromatic aberration and a soft drop shadow — all inside
//! one quad, so cards need no egui fill or shadow of their own.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use egui::{Rect, Shape};

/// True once the shaders compiled; cards fall back to an opaque fill otherwise.
static READY: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicBool = AtomicBool::new(false);
static RES: Mutex<Option<Res>> = Mutex::new(None);

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

pub fn available() -> bool {
    ENABLED.load(Ordering::Relaxed) && (READY.load(Ordering::Relaxed) || !FAILED.load(Ordering::Relaxed))
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
    let Some(p) = settings_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let v = serde_json::json!({ "glass": enabled(), "glass_strength": strength(), "shadow": shadow(), "glass_band": band(), "palette_near": palette_near() });
    let _ = std::fs::write(p, serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// Blur reach beyond the card edge, in points (captured margin).
const MARGIN: f32 = 36.0;
/// Shadow reach in points.
const SHADOW: f32 = 22.0;

#[derive(Clone, Copy)]
pub struct Look {
    pub radius: f32,
    /// Max sample offset at the rim, points.
    pub refract: f32,
    /// Tint colour (straight RGBA 0..1) laid over the blur.
    pub tint: [f32; 4],
    /// Red/blue split at the rim, points.
    pub dispersion: f32,
    /// Frame id; matches the scene snapshot taken this frame (see `scene_marker`).
    pub frame: u64,
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
    let cb = egui_glow::CallbackFn::new(move |info, painter| {
        // SAFETY: called by egui_glow on the GL thread with its context current.
        unsafe { paint(painter.gl(), &info, card, look) };
    });
    Shape::Callback(egui::PaintCallback { rect: card.expand(SHADOW + 8.0), callback: std::sync::Arc::new(cb) })
}

/// Render layers: everything painted before this marker is the *scene* layer
/// (gets tint, refraction and dispersion under glass); everything painted after it
/// and before a glass card is the *overlay* layer (only blurred, full brightness).
pub fn scene_marker(screen: Rect, frame: u64) -> Shape {
    let cb = egui_glow::CallbackFn::new(move |info, painter| {
        // SAFETY: called by egui_glow on the GL thread with its context current.
        unsafe { snapshot(painter.gl(), &info, frame) };
    });
    Shape::Callback(egui::PaintCallback { rect: screen, callback: std::sync::Arc::new(cb) })
}

unsafe fn snapshot(gl: &glow::Context, info: &egui::PaintCallbackInfo, frame: u64) {
    unsafe {
        if FAILED.load(Ordering::Relaxed) {
            return;
        }
        let mut guard = RES.lock().unwrap();
        let Some(res) = guard.as_mut() else { return };
        let [sw, sh] = info.screen_size_px;
        let (sw, sh) = (sw as i32, sh as i32);
        if ensure(gl, &mut res.scene, sw, sh).is_none() {
            return;
        }
        let scene = res.scene.as_ref().unwrap();
        gl.disable(glow::SCISSOR_TEST);
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(scene.fbo));
        gl.blit_framebuffer(0, 0, sw, sh, 0, 0, sw, sh, glow::COLOR_BUFFER_BIT, glow::NEAREST);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        res.scene_frame = frame;
    }
}

struct Tex {
    tex: glow::Texture,
    fbo: glow::Framebuffer,
    w: i32,
    h: i32,
}

struct Res {
    blur: glow::Program,
    comp: glow::Program,
    vao: glow::VertexArray,
    sharp: Option<Tex>,
    small: Option<Tex>,
    tmp: Option<Tex>,
    /// Region with the overlay layer included.
    sharp1: Option<Tex>,
    /// Full-screen snapshot of the scene layer (before overlays), and its frame id.
    scene: Option<Tex>,
    scene_frame: u64,
}

unsafe fn compile(gl: &glow::Context, header: &str, vs: &str, fs: &str) -> Result<glow::Program, String> {
    unsafe {
        let prog = gl.create_program()?;
        let mut shaders = Vec::new();
        for (ty, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
            let sh = gl.create_shader(ty)?;
            gl.shader_source(sh, &format!("{header}{src}"));
            gl.compile_shader(sh);
            if !gl.get_shader_compile_status(sh) {
                return Err(gl.get_shader_info_log(sh));
            }
            gl.attach_shader(prog, sh);
            shaders.push(sh);
        }
        gl.link_program(prog);
        if !gl.get_program_link_status(prog) {
            return Err(gl.get_program_info_log(prog));
        }
        for sh in shaders {
            gl.detach_shader(prog, sh);
            gl.delete_shader(sh);
        }
        Ok(prog)
    }
}

const VS_FULL: &str = r#"
out vec2 v_uv;
void main() {
    vec2 c = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    v_uv = c;
    gl_Position = vec4(c * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const FS_BLUR: &str = r#"
uniform sampler2D u_src;
uniform vec2 u_dir;
uniform vec2 u_uv_scale;   // used part of the (oversized) texture
in vec2 v_uv;
out vec4 o;
vec4 tap(vec2 uv) { return texture(u_src, clamp(uv, vec2(0.0), u_uv_scale - u_dir * 0.0 - vec2(1e-4))); }
void main() {
    float w[5] = float[](0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);
    vec2 uv = v_uv * u_uv_scale;
    vec4 acc = tap(uv) * w[0];
    for (int i = 1; i < 5; i++) {
        vec2 d = u_dir * float(i);
        acc += (tap(uv + d) + tap(uv - d)) * w[i];
    }
    o = acc;
}
"#;

const VS_QUAD: &str = r#"
uniform vec4 u_quad;   // x0 y0 x1 y1, GL pixels
uniform vec2 u_screen;
void main() {
    vec2 c = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    vec2 p = mix(u_quad.xy, u_quad.zw, c);
    gl_Position = vec4(p / u_screen * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const FS_COMP: &str = r#"
uniform sampler2D u_sharp;
uniform sampler2D u_blur;
uniform vec4 u_region;   // x y w h of the captured area, GL pixels
uniform vec4 u_panel;    // centre x y, half w h, GL pixels
uniform vec4 u_tint;
uniform float u_radius;
uniform float u_band;
uniform float u_refract;
uniform float u_shadow;
uniform float u_shadow_on;
uniform float u_ppp;
uniform vec2 u_blur_scale;
uniform vec2 u_sharp_max;
uniform float u_ca;          // dispersion at the rim, pixels
uniform sampler2D u_sharp1;  // region *with* the overlay layer
uniform float u_has_overlay;
out vec4 o;

float sd(vec2 p) {
    vec2 b = u_panel.zw;
    float r = u_radius;
    vec2 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec2 frag = gl_FragCoord.xy;
    vec2 p = frag - u_panel.xy;
    float d = sd(p);
    if (d > 0.0) {
        if (u_shadow_on < 0.5) { o = vec4(0.0); return; }
        // soft drop shadow, nudged downwards (GL y is up)
        float ds = sd(p + vec2(0.0, 6.0 * u_ppp));
        float t = clamp(1.0 - ds / u_shadow, 0.0, 1.0);
        float a = 0.38 * t * t * clamp(d + 0.5, 0.0, 1.0);
        o = vec4(0.0, 0.0, 0.0, a);
        return;
    }
    vec2 g = vec2(sd(p + vec2(1.0, 0.0)) - sd(p - vec2(1.0, 0.0)), sd(p + vec2(0.0, 1.0)) - sd(p - vec2(0.0, 1.0)));
    vec2 n = g / max(length(g), 1e-4);
    float edge = clamp(1.0 + d / u_band, 0.0, 1.0);
    float bend = edge * edge * u_refract;
    vec2 base = frag - n * bend;           // lens: pull samples inward at the rim
    vec2 px = 1.0 / u_region.zw;
    vec2 uv = clamp((base - u_region.xy) * px, vec2(0.0), u_sharp_max);
    // dispersion: red and blue split along the rim normal, strongest at the very edge,
    // applied to both the sharp and the blurred layer so the fringe survives the blur
    vec2 ca = n * edge * edge * u_ca * px;
    vec2 ur = clamp(uv + ca, vec2(0.0), u_sharp_max);
    vec2 ub = clamp(uv - ca, vec2(0.0), u_sharp_max);
    vec3 sharp = vec3(texture(u_sharp, ur).r, texture(u_sharp, uv).g, texture(u_sharp, ub).b);
    vec3 blur = vec3(texture(u_blur, ur * u_blur_scale).r, texture(u_blur, uv * u_blur_scale).g, texture(u_blur, ub * u_blur_scale).b);
    vec3 col = mix(blur, sharp, edge * 0.45);
    col = mix(col, u_tint.rgb, u_tint.a * (1.0 - edge * 0.35));
    if (u_has_overlay > 0.5) {
        // overlay layer = (with overlay) - (scene only), lightly blurred; the difference is
        // the overlay alone, added at full strength: no tint, refraction or dispersion
        vec2 us = (frag - u_region.xy) * px;
        vec3 od = vec3(0.0);
        float ws = 0.0;
        for (int i = -2; i <= 2; i++) {
            for (int j = -2; j <= 2; j++) {
                vec2 q = clamp(us + vec2(float(i), float(j)) * 1.5 * u_ppp * px, vec2(0.0), u_sharp_max);
                float wt = exp(-float(i * i + j * j) / 3.0);
                od += (texture(u_sharp1, q).rgb - texture(u_sharp, q).rgb) * wt;
                ws += wt;
            }
        }
        col = clamp(col + od / ws, 0.0, 1.0);
    }
    float a = clamp(0.5 - d, 0.0, 1.0);
    o = vec4(col * a, a);
}
"#;

unsafe fn ensure(gl: &glow::Context, slot: &mut Option<Tex>, w: i32, h: i32) -> Option<()> {
    unsafe {
        if let Some(t) = slot {
            if t.w >= w && t.h >= h {
                return Some(());
            }
            gl.delete_texture(t.tex);
            gl.delete_framebuffer(t.fbo);
            *slot = None;
        }
        // grow in steps so window resizes don't reallocate every frame
        let (aw, ah) = ((w + 255) & !255, (h + 255) & !255);
        let tex = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, aw, ah, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        let fbo = gl.create_framebuffer().ok()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
        *slot = Some(Tex { tex, fbo, w: aw, h: ah });
        Some(())
    }
}

unsafe fn init(gl: &glow::Context) -> Option<Res> {
    unsafe {
        let ver = egui_glow::ShaderVersion::get(gl);
        if !ver.is_new_shader_interface() {
            return None;
        }
        let mut header = ver.version_declaration().to_string();
        if ver.is_embedded() {
            header.push_str("precision highp float;\n");
        }
        let blur = compile(gl, &header, VS_FULL, FS_BLUR).map_err(|e| eprintln!("glass blur: {e}")).ok()?;
        let comp = compile(gl, &header, VS_QUAD, FS_COMP).map_err(|e| eprintln!("glass comp: {e}")).ok()?;
        let vao = gl.create_vertex_array().ok()?;
        Some(Res { blur, comp, vao, sharp: None, small: None, tmp: None, sharp1: None, scene: None, scene_frame: u64::MAX })
    }
}

unsafe fn paint(gl: &glow::Context, info: &egui::PaintCallbackInfo, card: Rect, look: Look) {
    unsafe {
        if FAILED.load(Ordering::Relaxed) {
            return;
        }
        let mut guard = RES.lock().unwrap();
        if guard.is_none() {
            match init(gl) {
                Some(r) => {
                    *guard = Some(r);
                    READY.store(true, Ordering::Relaxed);
                }
                None => {
                    FAILED.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
        let res = guard.as_mut().unwrap();
        let ppp = info.pixels_per_point;
        let [sw, sh] = info.screen_size_px;
        let (sw, sh) = (sw as i32, sh as i32);

        // captured region (card + blur margin), clamped to the screen, in GL pixels
        let reg = card.expand(MARGIN);
        let x0 = ((reg.min.x * ppp).floor() as i32).clamp(0, sw);
        let x1 = ((reg.max.x * ppp).ceil() as i32).clamp(0, sw);
        let ytop = ((reg.min.y * ppp).floor() as i32).clamp(0, sh);
        let ybot = ((reg.max.y * ppp).ceil() as i32).clamp(0, sh);
        let (w, h) = (x1 - x0, ybot - ytop);
        if w < 8 || h < 8 {
            return;
        }
        let gy0 = sh - ybot;
        let (sw4, sh4) = ((w / 4).max(2), (h / 4).max(2));
        let has_overlay = res.scene_frame == look.frame && res.scene.as_ref().is_some_and(|t| t.w >= sw && t.h >= sh);
        if ensure(gl, &mut res.sharp, w, h).is_none()
            || ensure(gl, &mut res.small, sw4, sh4).is_none()
            || ensure(gl, &mut res.tmp, sw4, sh4).is_none()
            || (has_overlay && ensure(gl, &mut res.sharp1, w, h).is_none())
        {
            return;
        }
        let (sharp, small, tmp) = (res.sharp.as_ref().unwrap(), res.small.as_ref().unwrap(), res.tmp.as_ref().unwrap());

        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::BLEND);
        // 1. `sharp` = scene layer only (from the snapshot if there is one), and
        //    `sharp1` = what is on screen now (scene + overlay layer)
        let scene_src = if has_overlay { res.scene.as_ref().map(|t| t.fbo) } else { None };
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, scene_src);
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(sharp.fbo));
        gl.blit_framebuffer(x0, gy0, x1, gy0 + h, 0, 0, w, h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
        if has_overlay {
            let sharp1 = res.sharp1.as_ref().unwrap();
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(sharp1.fbo));
            gl.blit_framebuffer(x0, gy0, x1, gy0 + h, 0, 0, w, h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
        }
        // 2-3. quarter-size separable gaussian of each layer
        let blur_into = |src_full: &Tex, small: &Tex| {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(src_full.fbo));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(small.fbo));
            gl.blit_framebuffer(0, 0, w, h, 0, 0, sw4, sh4, glow::COLOR_BUFFER_BIT, glow::LINEAR);
            gl.use_program(Some(res.blur));
            gl.bind_vertex_array(Some(res.vao));
            gl.active_texture(glow::TEXTURE0);
            gl.uniform_1_i32(gl.get_uniform_location(res.blur, "u_src").as_ref(), 0);
            let u_dir = gl.get_uniform_location(res.blur, "u_dir");
            // uv covers only the used part of the (larger) textures
            let (fx, fy) = (sw4 as f32 / small.w as f32, sh4 as f32 / small.h as f32);
            for _ in 0..2 {
                for (src, dst, dir) in [(small, tmp, [1.4 / small.w as f32, 0.0]), (tmp, small, [0.0, 1.4 / small.h as f32])] {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(dst.fbo));
                    gl.bind_texture(glow::TEXTURE_2D, Some(src.tex));
                    gl.uniform_2_f32(u_dir.as_ref(), dir[0], dir[1]);
                    gl.viewport(0, 0, sw4, sh4);
                    set_uv_scale(gl, res.blur, fx, fy);
                    gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                }
            }
        };
        blur_into(sharp, small);

        // 4. composite onto the screen
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.viewport(0, 0, sw, sh);
        gl.enable(glow::BLEND);
        gl.blend_equation(glow::FUNC_ADD);
        gl.blend_func_separate(glow::ONE, glow::ONE_MINUS_SRC_ALPHA, glow::ONE_MINUS_DST_ALPHA, glow::ONE);
        let clip = info.clip_rect_in_pixels();
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(clip.left_px, clip.from_bottom_px, clip.width_px, clip.height_px);
        gl.use_program(Some(res.comp));
        let u = |n: &str| gl.get_uniform_location(res.comp, n);
        let q = card.expand(SHADOW + 8.0);
        gl.uniform_4_f32(u("u_quad").as_ref(), q.min.x * ppp, sh as f32 - q.max.y * ppp, q.max.x * ppp, sh as f32 - q.min.y * ppp);
        gl.uniform_2_f32(u("u_screen").as_ref(), sw as f32, sh as f32);
        // region in GL pixels, expressed against the full allocated textures
        gl.uniform_4_f32(u("u_region").as_ref(), x0 as f32, gy0 as f32, sharp.w as f32, sharp.h as f32);
        let c = card.center();
        gl.uniform_4_f32(u("u_panel").as_ref(), c.x * ppp, sh as f32 - c.y * ppp, card.width() * 0.5 * ppp, card.height() * 0.5 * ppp);
        gl.uniform_4_f32(u("u_tint").as_ref(), look.tint[0], look.tint[1], look.tint[2], look.tint[3]);
        gl.uniform_1_f32(u("u_radius").as_ref(), look.radius * ppp);
        gl.uniform_1_f32(u("u_band").as_ref(), band() * ppp);
        let k = strength();
        gl.uniform_1_f32(u("u_refract").as_ref(), look.refract * ppp * k);
        gl.uniform_1_f32(u("u_ca").as_ref(), look.dispersion * ppp * k);
        gl.uniform_1_f32(u("u_shadow").as_ref(), SHADOW * ppp);
        gl.uniform_1_f32(u("u_shadow_on").as_ref(), if shadow() { 1.0 } else { 0.0 });
        gl.uniform_1_f32(u("u_ppp").as_ref(), ppp);
        gl.uniform_1_f32(u("u_has_overlay").as_ref(), if has_overlay { 1.0 } else { 0.0 });
        if has_overlay {
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, Some(res.sharp1.as_ref().unwrap().tex));
            gl.uniform_1_i32(u("u_sharp1").as_ref(), 2);
            gl.active_texture(glow::TEXTURE0);
        }
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(sharp.tex));
        gl.uniform_1_i32(u("u_sharp").as_ref(), 0);
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, Some(small.tex));
        gl.uniform_1_i32(u("u_blur").as_ref(), 1);
        // the blurred texture holds a quarter-size copy: scale its uv to match
        gl.uniform_2_f32(u("u_blur_scale").as_ref(), (sw4 as f32 / small.w as f32) / (w as f32 / sharp.w as f32), (sh4 as f32 / small.h as f32) / (h as f32 / sharp.h as f32));
        gl.uniform_2_f32(u("u_sharp_max").as_ref(), (w as f32 - 1.0) / sharp.w as f32, (h as f32 - 1.0) / sharp.h as f32);
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_vertex_array(None);
    }
}

unsafe fn set_uv_scale(gl: &glow::Context, prog: glow::Program, fx: f32, fy: f32) {
    unsafe {
        gl.uniform_2_f32(gl.get_uniform_location(prog, "u_uv_scale").as_ref(), fx, fy);
    }
}
