//! GPU export (render overhaul phase 2, docs/updates/wgpu-renderer.md).
//!
//! Draws a frame description (`scene.rs`) on a headless wgpu device. Each 2D
//! layer is still rasterized by the CPU code (tiny-skia, `export::draw_layer`),
//! but only into its own box, aligned to the output pixels, and kept on the GPU
//! across frames while it doesn't change. Projection warps, extrusion caps,
//! compositing, blend modes, masks and supersampling run on the GPU. The CPU
//! export (`export.rs`) stays as the reference (golden images, parity tests,
//! `DV_CPU_EXPORT=1`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use eframe::wgpu;
use egui::{Color32, Pos2, Rect, vec2};
use tiny_skia::{Pixmap, Transform};
use wgpu::util::DeviceExt;

use crate::doc::{Blend, Document, Layer, LayerId};
use crate::export::{FlatCache, MaskMap, output_size};
use crate::scene::{Draw, Projected};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A picture queued on the GPU (`Gpu::submit`), collected by `Gpu::finish`.
pub struct Pending {
    buf: wgpu::Buffer,
    kind: Kind,
    w: u32,
    h: u32,
    index: wgpu::SubmissionIndex,
    rx: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

enum Kind {
    Rgba { bpr: u32 },
    Nv12 { bpr: u32, uv_off: u64 },
}

/// A finished picture: premultiplied RGBA, or NV12 bytes (luma, then
/// interleaved chroma; BT.709 limited range) with its size.
pub enum Frame {
    Rgba(Pixmap),
    Nv12(Vec<u8>, u32, u32),
}

/// NV12 (BT.709, limited range) of an opaque premultiplied picture, as the GPU's
/// `fs_y` / `fs_uv` make it (for frames the CPU drew).
pub fn nv12_of(pm: &Pixmap) -> Vec<u8> {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let d = pm.data();
    let mut out = vec![0u8; w * h + w * (h / 2)];
    let (ys, uvs) = out.split_at_mut(w * h);
    let px = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        [d[i] as f32 / 255.0, d[i + 1] as f32 / 255.0, d[i + 2] as f32 / 255.0]
    };
    for (y, row) in ys.chunks_mut(w).enumerate() {
        for (x, v) in row.iter_mut().enumerate() {
            let c = px(x, y);
            *v = (16.0 + 219.0 * (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2])).round().clamp(0.0, 255.0) as u8;
        }
    }
    for (y, row) in uvs.chunks_mut(w).enumerate().take(h / 2) {
        for x in 0..w / 2 {
            let mut c = [0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = px(x * 2 + dx, y * 2 + dy);
                for k in 0..3 {
                    c[k] += p[k] * 0.25;
                }
            }
            row[x * 2] = (128.0 + 224.0 * (-0.114572 * c[0] - 0.385428 * c[1] + 0.5 * c[2])).round().clamp(0.0, 255.0) as u8;
            row[x * 2 + 1] = (128.0 + 224.0 * (0.5 * c[0] - 0.454153 * c[1] - 0.045847 * c[2])).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// One picture to draw, prepared on the CPU side.
enum Node {
    /// A picture at `off` (target pixels) put over the target.
    Over { img: Img, off: [i32; 2], opacity: f32 },
    /// A photo or video frame through `inv` (target pixel -> source pixel).
    Image { img: Img, inv: [[f32; 3]; 2], rect: [i32; 4], opacity: f32, nearest: bool, px_per_src: f32 },
    /// A flattened layer through `hi` (inverse homography, target -> canvas).
    Warp { img: Img, hi: [[f32; 3]; 3], src: [f32; 3], rect: [i32; 4], opacity: f32, shade: f32 },
    /// Text with a blend mode.
    Blend { img: Img, off: [i32; 2], mode: Blend, opacity: f32 },
    /// Drawn alone into a target of its own, then blended down once.
    Group { inner: Vec<Node>, rect: [i32; 4], mode: Blend, opacity: f32 },
    /// Extrusion side triangles (x, y, premultiplied rgba; pixels of `rect`'s
    /// box), drawn multisampled then put over the target; `scissor` in box pixels.
    Sides { verts: Vec<[f32; 6]>, rect: [i32; 4], scissor: [u32; 4] },
    /// Drawn alone, then multiplied by a mask's keep values.
    Masked { inner: Vec<Node>, keep: Img, off: [i32; 2], rect: [i32; 4], outside: f32 },
}

/// A texture: cached, or still to be rasterized (index into the job list).
enum Img {
    Ready(Arc<wgpu::Texture>),
    Job(usize),
}

type Raster = Box<dyn FnOnce() -> Option<(Px, u32, u32)> + Send>;

/// Pixels to upload: made for this picture, or an opaque video frame as it came
/// from the decoder (no copy).
enum Px {
    Own(Vec<u8>),
    Shared(Arc<image::RgbaImage>),
    /// A decoded video frame, turned into RGBA on the GPU.
    Nv12(Arc<crate::doc::Nv12>),
}

impl std::ops::Deref for Px {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Px::Own(v) => v,
            Px::Shared(i) => i.as_raw(),
            Px::Nv12(f) => &f.data,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    id: u64,
    content: u64,
    rect: [i32; 4],
    view: [u32; 4],
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    over: wgpu::RenderPipeline,
    warp: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    mask: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    tri: wgpu::RenderPipeline,
    image: wgpu::RenderPipeline,
    y_plane: wgpu::RenderPipeline,
    uv_plane: wgpu::RenderPipeline,
    from_nv12: wgpu::RenderPipeline,
    ground: wgpu::RenderPipeline,
    dummy: wgpu::Texture,
    pub max_side: u32,
    pub adapter: String,
    /// PCI vendor id (0x10de: NVIDIA).
    pub vendor: u32,
    /// Flattened 3D layers, kept across a job's frames.
    flat: Arc<FlatCache>,
    /// Rasterized 2D layers and mask keep images, by content and placement.
    rasters: HashMap<Key, (Arc<wgpu::Texture>, u64)>,
    /// Textures of flattened 3D layers (the entry is kept so its address stays unique).
    flats: HashMap<usize, (Arc<(Pixmap, Rect, f32, Option<Arc<Vec<crate::extrude::Edge>>>)>, Arc<wgpu::Texture>, u64)>,
    /// Extrusion sides in canvas units, by outline, placement, camera and light:
    /// panning only moves them, and a still layer doesn't recompute them.
    sides: HashMap<u64, (Arc<Vec<crate::extrude::Side>>, u64)>,
    /// Full-size targets: (w, h, slot).
    pool: HashMap<(u32, u32, usize), wgpu::Texture>,
    /// Brush layers on the canvas: their tiles in one texture each, updated tile
    /// by tile as they change (painting re-uploads only the touched tiles).
    atlases: HashMap<LayerId, Atlas>,
    frame: u64,
}

static GPU: Mutex<Option<Option<Gpu>>> = Mutex::new(None);

/// The window's wgpu device (eframe on wgpu): exports, the playback cache and
/// the canvas all use it, so textures could be shared and nothing is made twice.
#[derive(Clone)]
struct Shared {
    device: wgpu::Device,
    queue: wgpu::Queue,
    max_side: u32,
    adapter: String,
    vendor: u32,
}

static SHARED: Mutex<Option<Shared>> = Mutex::new(None);

/// Called once at start when eframe runs on wgpu.
pub fn install(rs: &eframe::egui_wgpu::RenderState) {
    let info = rs.adapter.get_info();
    let max_side = rs.device.limits().max_texture_dimension_2d;
    if let Ok(mut s) = SHARED.lock() {
        *s = Some(Shared { device: rs.device.clone(), queue: rs.queue.clone(), max_side, adapter: info.name, vendor: info.vendor });
    }
}

thread_local! {
    /// DV_TIMING: milliseconds spent preparing (scene, flattening), rasterizing,
    /// encoding the GPU work, and waiting for it and reading back, summed over a
    /// job's frames.
    static TIMING: std::cell::Cell<bool> = std::cell::Cell::new(std::env::var_os("DV_TIMING").is_some());
    static STATS: std::cell::Cell<[f64; 4]> = const { std::cell::Cell::new([0.0; 4]) };
}

/// The summed stage times (ms) since the last call, and resets them.
pub fn take_stats() -> [f64; 4] {
    STATS.with(|c| c.replace([0.0; 4]))
}

/// Runs `f` with the shared GPU device (made on first use). None: no usable GPU,
/// or `DV_CPU_EXPORT` is set.
pub fn with_gpu<R>(f: impl FnOnce(&mut Gpu) -> R) -> Option<R> {
    if std::env::var_os("DV_CPU_EXPORT").is_some() {
        return None;
    }
    let mut g = GPU.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(match Gpu::new() {
            Ok(gpu) => Some(gpu),
            Err(e) => {
                eprintln!("GPU export unavailable: {e}");
                None
            }
        });
    }
    let gpu = g.as_mut().unwrap().as_mut()?;
    // a GPU error (device lost, a driver fault) must not take the export down:
    // the GPU is switched off for this run and the caller uses the CPU
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(gpu))) {
        Ok(r) => Some(r),
        Err(_) => {
            eprintln!("GPU export failed; using the CPU from now on");
            *g = Some(None);
            None
        }
    }
}

/// An export picture (`export::render_ss`) on the GPU when there is one, else on
/// the CPU. `keep`: the layers stay cached for the next frame (video export);
/// otherwise they are dropped afterwards.
#[allow(clippy::too_many_arguments)]
pub fn render_ss(doc: &Document, center: Pos2, size: egui::Vec2, rot: f32, scale: f32, bg: Option<Color32>, k: u8, keep: bool) -> Result<Pixmap, String> {
    let r = with_gpu(|g| {
        let r = g.render_ss(doc, center, size, rot, scale, bg, k);
        if !keep {
            g.clear();
        }
        r
    });
    match r {
        Some(Ok(pm)) => Ok(pm),
        _ => crate::export::render_ss(doc, center, size, rot, scale, bg, k),
    }
}

/// Drops what the GPU keeps between frames (start and end of a video export).
pub fn end_job() {
    if let Ok(mut g) = GPU.lock() {
        if let Some(Some(g)) = g.as_mut() {
            g.clear();
        }
    }
}

impl Gpu {
    /// A renderer on the window's device (None under glow): the canvas has its
    /// own, so its caches and the export's don't push each other out.
    pub fn on_window_device() -> Option<Gpu> {
        let s = SHARED.lock().ok()?.clone()?;
        Some(Gpu::build(s.device, s.queue, s.max_side, s.adapter, s.vendor))
    }

    fn new() -> Result<Gpu, String> {
        if let Some(s) = SHARED.lock().ok().and_then(|s| s.clone()) {
            return Ok(Gpu::build(s.device, s.queue, s.max_side, s.adapter, s.vendor));
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|e| e.to_string())?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            return Err("software adapter".into());
        }
        let limits = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("export"),
            required_limits: wgpu::Limits { max_texture_dimension_2d: limits.max_texture_dimension_2d, ..wgpu::Limits::downlevel_defaults() },
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        Ok(Gpu::build(device, queue, limits.max_texture_dimension_2d, info.name, info.vendor))
    }

    fn build(device: wgpu::Device, queue: wgpu::Queue, max_side: u32, adapter: String, vendor: u32) -> Gpu {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("export"), source: wgpu::ShaderSource::Wgsl(include_str!("gpu.wgsl").into()) });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let over_blend = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
        };
        let pipe_fmt = |entry: &str, blend: Option<wgpu::BlendState>, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipe = |entry: &str, blend: Option<wgpu::BlendState>| pipe_fmt(entry, blend, FMT);
        let y_plane = pipe_fmt("fs_y", None, wgpu::TextureFormat::R8Unorm);
        let uv_plane = pipe_fmt("fs_uv", None, wgpu::TextureFormat::Rg8Unorm);
        let from_nv12 = pipe("fs_from_nv12", None);
        let ground = pipe("fs_ground", None);
        let over = pipe("fs_over", Some(over_blend));
        let warp = pipe("fs_warp", Some(over_blend));
        let blend = pipe("fs_blend", None);
        let mask = pipe("fs_mask", Some(over_blend));
        let down = pipe("fs_down", None);
        let image = pipe("fs_image", Some(over_blend));
        let tri = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sides"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_tri"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 24,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_tri"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: FMT, blend: Some(over_blend), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: 4, ..Default::default() },
            multiview_mask: None,
            cache: None,
        });
        let dummy = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FMT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Gpu {
            device,
            queue,
            layout,
            over,
            warp,
            blend,
            mask,
            down,
            tri,
            image,
            y_plane,
            uv_plane,
            from_nv12,
            ground,
            dummy,
            max_side,
            adapter,
            vendor,
            flat: Arc::default(),
            rasters: HashMap::new(),
            flats: HashMap::new(),
            pool: HashMap::new(),
            atlases: HashMap::new(),
            sides: HashMap::new(),
            frame: 0,
        }
    }

    /// Drops everything kept between frames (after a job).
    pub fn clear(&mut self) {
        self.atlases.clear();
        self.sides.clear();
        self.flat = Arc::default();
        self.rasters.clear();
        self.flats.clear();
        self.pool.clear();
    }

    fn texture(&self, w: u32, h: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FMT,
            usage,
            view_formats: &[],
        })
    }

    fn upload(&self, data: &[u8], w: u32, h: u32) -> Arc<wgpu::Texture> {
        let t = self.texture(w, h, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        Arc::new(t)
    }

    /// An NV12 frame uploaded as one R8 texture (luma rows, then chroma rows)
    /// and turned into a premultiplied RGBA texture on the GPU.
    fn upload_nv12(&self, data: &[u8], w: u32, h: u32) -> Arc<wgpu::Texture> {
        let rows = h + h / 2;
        let src = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &src, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(rows) },
            wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
        );
        let out = self.texture(w, h, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let mut enc = self.device.create_command_encoder(&Default::default());
        let mut u = Uniform::new([0.0, 0.0, w as f32, h as f32], w, h);
        u.p1 = [h as f32, 0.0, 0.0, 0.0];
        self.pass(&mut enc, &self.from_nv12, &out, &u, &src, &self.dummy);
        self.queue.submit([enc.finish()]);
        Arc::new(out)
    }

    fn pooled(&mut self, w: u32, h: u32, slot: usize) -> wgpu::Texture {
        self.pool
            .entry((w, h, slot))
            .or_insert_with(|| {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FMT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            })
            .clone()
    }

    /// As `export::render_ss`: the picture of `size` (canvas units) about
    /// `center`, turned `rot`, at `scale` output pixels per unit, supersampled
    /// `k`×. Err when it doesn't fit the GPU (the caller uses the CPU).
    #[allow(clippy::too_many_arguments)]
    pub fn render_ss(&mut self, doc: &Document, center: Pos2, size: egui::Vec2, rot: f32, scale: f32, bg: Option<Color32>, k: u8) -> Result<Pixmap, String> {
        let p = self.submit(doc, center, size, rot, scale, bg, k, false)?;
        match self.finish(p)? {
            Frame::Rgba(pm) => Ok(pm),
            Frame::Nv12(..) => Err("NV12".into()),
        }
    }

    /// Prepares and queues one picture on the GPU without waiting for it; `nv12`
    /// reads it back as NV12 (video) instead of RGBA. `finish` collects it, so
    /// the next frame can be prepared while the GPU draws this one.
    #[allow(clippy::too_many_arguments)]
    pub fn submit(&mut self, doc: &Document, center: Pos2, size: egui::Vec2, rot: f32, scale: f32, bg: Option<Color32>, k: u8, nv12: bool) -> Result<Pending, String> {
        let mut k = k.clamp(1, 4) as u32;
        let rotated = rot.abs() >= 1e-6;
        // the picture drawn (the box round a turned frame) at s = scale·k
        let geom = |k: u32| {
            let s = scale * k as f32;
            if rotated {
                let (sn, cs) = rot.sin_cos();
                let h = size * 0.5;
                let ext = vec2((h.x * cs).abs() + (h.y * sn).abs(), (h.x * sn).abs() + (h.y * cs).abs());
                let bb = Rect::from_center_size(center, ext * 2.0 + vec2(4.0, 4.0) / s);
                let big = ((size.x * s).round().max(1.0) as u32, (size.y * s).round().max(1.0) as u32);
                (s, bb, output_size(bb, s), big)
            } else {
                let r = Rect::from_center_size(center, size);
                let d = output_size(r, s);
                (s, r, d, d)
            }
        };
        while k > 1 && {
            let (_, _, (w, h), _) = geom(k);
            w > self.max_side || h > self.max_side
        } {
            k -= 1;
        }
        let (s, full, (tw, th), big) = geom(k);
        if tw > self.max_side || th > self.max_side {
            return Err(format!("GPU 한도 초과: {tw}×{th}px"));
        }
        let (fw, fh) = (big.0 / k, big.1 / k);
        self.frame += 1;
        let g = self.frame;
        self.rasters.retain(|_, v| v.1 + 1 >= g);
        self.flats.retain(|_, v| v.2 + 1 >= g);
        self.sides.retain(|_, v| v.1 + 1 >= g);
        self.flat.sweep();

        // 1. what to draw, with the rasterizing left as jobs
        let t0 = std::time::Instant::now();
        let scene = crate::scene::build(doc);
        let view = View { full, s, tw, th, k: k as f32, origin: None, zooming: false };
        let cull = full.expand(2.0 / s);
        let mut jobs: Vec<(Raster, Option<Key>)> = Vec::new();
        let mut nodes = Vec::new();
        for item in scene.items.into_iter().filter(|it| it.bounds.intersects(cull)) {
            let b = item.bounds;
            self.prepare(item.draw, b, &view, &mut jobs, &mut nodes);
        }

        // 2. rasterize on every core, then upload
        let t1 = std::time::Instant::now();
        let ready = self.run_jobs(jobs, view.k);

        // 3. draw
        let t2 = std::time::Instant::now();
        let target = self.pooled(tw, th, 0);
        let mut enc = self.device.create_command_encoder(&Default::default());
        let clear = bg.map_or(wgpu::Color::TRANSPARENT, |c| {
            let [r, gg, b, a] = c.to_srgba_unmultiplied().map(|v| v as f64 / 255.0);
            wgpu::Color { r: r * a, g: gg * a, b: b * a, a }
        });
        self.clear_to(&mut enc, &target, clear);
        let bgp = bg.map(|c| {
            let [r, gg, b, a] = c.to_srgba_unmultiplied().map(|v| v as f32 / 255.0);
            [r * a, gg * a, b * a, a]
        });
        for nd in &nodes {
            self.run(&mut enc, nd, &target, 0, &ready, &view, bgp);
        }
        // 4. down to the output size (and turned), then read back
        let out = self.texture(fw, fh, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        let mut u = Uniform::new([0.0, 0.0, fw as f32, fh as f32], fw, fh);
        u.p1[3] = k as f32;
        if rotated {
            let (sn, cs) = rot.sin_cos();
            u.rot = [cs, sn, big.0 as f32 * 0.5, big.1 as f32 * 0.5];
            u.rot2 = [(full.min.x - center.x) * s, (full.min.y - center.y) * s, 0.0, 0.0];
        } else {
            u.rot = [1.0, 0.0, 0.0, 0.0];
        }
        let nv12 = nv12 && fw % 2 == 0 && fh % 2 == 0;
        let out = if nv12 { self.texture(fw, fh, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING) } else { out };
        self.pass(&mut enc, &self.down, &out, &u, &target, &self.dummy);
        let copy = |enc: &mut wgpu::CommandEncoder, t: &wgpu::Texture, buf: &wgpu::Buffer, offset: u64, bpr: u32, w: u32, h: u32| {
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo { buffer: buf, layout: wgpu::TexelCopyBufferLayout { offset, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        };
        let read = |size: u64| self.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let (buf, kind) = if nv12 {
            // luma plane, then the interleaved chroma plane (rows padded to 256)
            let bpr = fw.div_ceil(256) * 256;
            let (cw, ch) = (fw / 2, fh / 2);
            let plane = |w: u32, h: u32, f: wgpu::TextureFormat| {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: f,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            };
            let (yt, uvt) = (plane(fw, fh, wgpu::TextureFormat::R8Unorm), plane(cw, ch, wgpu::TextureFormat::Rg8Unorm));
            self.pass(&mut enc, &self.y_plane, &yt, &Uniform::new([0.0, 0.0, fw as f32, fh as f32], fw, fh), &out, &self.dummy);
            self.pass(&mut enc, &self.uv_plane, &uvt, &Uniform::new([0.0, 0.0, cw as f32, ch as f32], cw, ch), &out, &self.dummy);
            let uv_off = bpr as u64 * fh as u64;
            let buf = read(uv_off + bpr as u64 * ch as u64);
            copy(&mut enc, &yt, &buf, 0, bpr, fw, fh);
            copy(&mut enc, &uvt, &buf, uv_off, bpr, cw, ch);
            (buf, Kind::Nv12 { bpr, uv_off })
        } else {
            let bpr = (fw * 4).div_ceil(256) * 256;
            let buf = read(bpr as u64 * fh as u64);
            copy(&mut enc, &out, &buf, 0, bpr, fw, fh);
            (buf, Kind::Rgba { bpr })
        };
        let index = self.queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        if TIMING.with(|c| c.get()) {
            let t3 = std::time::Instant::now();
            let ms = |a: std::time::Instant, b: std::time::Instant| (b - a).as_secs_f64() * 1000.0;
            STATS.with(|c| {
                let mut v = c.get();
                for (i, x) in [ms(t0, t1), ms(t1, t2), ms(t2, t3)].into_iter().enumerate() {
                    v[i] += x;
                }
                c.set(v);
            });
        }
        Ok(Pending { buf, kind, w: fw, h: fh, index, rx })
    }

    /// The canvas: the document as framed by the window (`world_min` at the
    /// target's top left, `s` pixels per canvas unit) drawn over the ground into
    /// `out` (Rgba8Unorm, the window's pixels). Nothing is read back; egui shows
    /// `out`. Returns the sub-pixel offset of the drawing (target pixels) to
    /// place it at, since the drawing snaps to whole canvas pixels.
    pub fn render_canvas(&mut self, doc: &Document, world_min: Pos2, s: f32, out: &wgpu::Texture, ground: &Ground, zooming: bool) -> egui::Vec2 {
        let (tw, th) = (out.width(), out.height());
        let ox = (world_min.x * s).floor();
        let oy = (world_min.y * s).floor();
        let frac = vec2(world_min.x * s - ox, world_min.y * s - oy);
        let full = Rect::from_min_size(Pos2::new(ox / s, oy / s), vec2(tw as f32 / s, th as f32 / s));
        self.frame += 1;
        let g = self.frame;
        // the canvas keeps pictures a little longer (a few frames of panning)
        self.rasters.retain(|_, v| v.1 + 30 >= g);
        self.flats.retain(|_, v| v.2 + 30 >= g);
        self.atlases.retain(|_, a| a.used + 30 >= g);
        self.sides.retain(|_, v| v.1 + 30 >= g);
        self.flat.sweep();
        let t0 = std::time::Instant::now();
        let scene = crate::scene::build(doc);
        let view = View { full, s, tw, th, k: 1.0, origin: Some([ox as i32, oy as i32]), zooming };
        let cull = full.expand(2.0 / s);
        let mut jobs: Vec<(Raster, Option<Key>)> = Vec::new();
        let mut nodes = Vec::new();
        for item in scene.items.into_iter().filter(|it| it.bounds.intersects(cull)) {
            let b = item.bounds;
            self.prepare(item.draw, b, &view, &mut jobs, &mut nodes);
        }
        let t1 = std::time::Instant::now();
        let ready = self.run_jobs(jobs, 1.0);
        let t2 = std::time::Instant::now();
        let mut enc = self.device.create_command_encoder(&Default::default());
        let mut u = Uniform::new([0.0, 0.0, tw as f32, th as f32], tw, th);
        u.h0 = ground.void;
        u.h1 = ground.warm;
        u.h2 = ground.cool;
        u.p0 = ground.warm_at;
        u.p1 = ground.cool_at;
        u.bg = ground.dot;
        // dots: step in target pixels, grid offset, radius
        let step = ground.step * s;
        u.rot = [step, (ox).rem_euclid(step), (oy).rem_euclid(step), ground.dot_r];
        self.pass(&mut enc, &self.ground, out, &u, &self.dummy, &self.dummy);
        for nd in &nodes {
            self.run(&mut enc, nd, out, 0, &ready, &view, None);
        }
        self.queue.submit([enc.finish()]);
        if TIMING.with(|c| c.get()) {
            let t3 = std::time::Instant::now();
            let ms = |a: std::time::Instant, b: std::time::Instant| (b - a).as_secs_f64() * 1000.0;
            STATS.with(|c| {
                let mut v = c.get();
                for (i, x) in [ms(t0, t1), ms(t1, t2), ms(t2, t3)].into_iter().enumerate() {
                    v[i] += x;
                }
                c.set(v);
            });
        }
        frac
    }

    /// Runs rasterizing jobs on every core and uploads the results (cached by
    /// their keys).
    fn run_jobs(&mut self, jobs: Vec<(Raster, Option<Key>)>, k: f32) -> Vec<Arc<wgpu::Texture>> {
        let g = self.frame;
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let n = jobs.len();
        let (rasters, keys): (Vec<Raster>, Vec<Option<Key>>) = jobs.into_iter().unzip();
        let queue: Mutex<Vec<(usize, Raster)>> = Mutex::new(rasters.into_iter().enumerate().rev().collect());
        let done: Mutex<Vec<Option<(Px, u32, u32)>>> = Mutex::new((0..n).map(|_| None).collect());
        std::thread::scope(|sc| {
            for _ in 0..threads.min(n) {
                sc.spawn(|| {
                    crate::export::set_ss(k);
                    loop {
                        let Some((i, job)) = queue.lock().unwrap().pop() else { break };
                        let r = job();
                        done.lock().unwrap()[i] = r;
                    }
                });
            }
        });
        let mut ready: Vec<Arc<wgpu::Texture>> = Vec::with_capacity(n);
        for (r, key) in done.into_inner().unwrap().into_iter().zip(keys) {
            let t = match r {
                Some((Px::Nv12(f), w, h)) => self.upload_nv12(&f.data, w, h),
                Some((data, w, h)) => self.upload(&data, w, h),
                None => Arc::new(self.texture(1, 1, wgpu::TextureUsages::TEXTURE_BINDING)),
            };
            if let Some(key) = key {
                self.rasters.insert(key, (t.clone(), g));
            }
            ready.push(t);
        }
        ready
    }

    /// Waits for a queued picture and reads it back.
    pub fn finish(&self, p: Pending) -> Result<Frame, String> {
        let t = std::time::Instant::now();
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: Some(p.index), timeout: None });
        p.rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        let (w, h) = (p.w, p.h);
        let frame = {
            let m = p.buf.slice(..).get_mapped_range().map_err(|e| e.to_string())?;
            match p.kind {
                Kind::Rgba { bpr } => {
                    let mut data = Vec::with_capacity((w * h * 4) as usize);
                    for row in m.chunks(bpr as usize).take(h as usize) {
                        data.extend_from_slice(&row[..(w * 4) as usize]);
                    }
                    Frame::Rgba(Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h).ok_or("크기 오류")?).ok_or("이미지 버퍼를 만들 수 없음")?)
                }
                Kind::Nv12 { bpr, uv_off } => {
                    let mut data = Vec::with_capacity((w * h * 3 / 2) as usize);
                    for row in m[..uv_off as usize].chunks(bpr as usize).take(h as usize) {
                        data.extend_from_slice(&row[..w as usize]);
                    }
                    for row in m[uv_off as usize..].chunks(bpr as usize).take((h / 2) as usize) {
                        data.extend_from_slice(&row[..w as usize]);
                    }
                    Frame::Nv12(data, w, h)
                }
            }
        };
        p.buf.unmap();
        if TIMING.with(|c| c.get()) {
            STATS.with(|c| {
                let mut v = c.get();
                v[3] += t.elapsed().as_secs_f64() * 1000.0;
                c.set(v);
            });
        }
        Ok(frame)
    }

    /// Turns one draw item into nodes; rasterizing is queued in `jobs`.
    fn prepare(&mut self, d: Draw, bounds: Rect, v: &View, jobs: &mut Vec<(Raster, Option<Key>)>, out: &mut Vec<Node>) {
        match d {
            Draw::Flat(layer) if matches!(layer.kind, crate::doc::LayerKind::Image(_)) => self.image(layer, v, jobs, out),
            Draw::Flat(layer) if v.origin.is_some() && matches!(layer.kind, crate::doc::LayerKind::Raster(_)) => {
                if let Some(n) = self.atlas(&layer, v) {
                    out.push(n);
                }
            }
            Draw::Flat(layer) => {
                let opacity = layer.opacity;
                if let Some((img, off)) = self.raster(layer, bounds, v, jobs) {
                    out.push(Node::Over { img, off, opacity });
                }
            }
            Draw::Blend(layer, mode) => {
                let opacity = layer.opacity;
                if let Some((img, off)) = self.raster(layer, bounds, v, jobs) {
                    out.push(Node::Blend { img, off, mode, opacity });
                }
            }
            Draw::Group { inner, mode, opacity } => {
                let mut nodes = Vec::new();
                self.prepare(*inner, bounds, v, jobs, &mut nodes);
                if let Some(rect) = v.px(bounds) {
                    out.push(Node::Group { inner: nodes, rect, mode, opacity });
                }
            }
            Draw::Masked { inner, mask, map } => {
                let mut nodes = Vec::new();
                self.prepare(*inner, bounds, v, jobs, &mut nodes);
                let Some(rect) = v.px(bounds) else { return };
                let strength = mask.opacity.clamp(0.0, 1.0);
                let outside = 1.0 - if mask.mask_invert { strength } else { 0.0 };
                let keep = self.keep(mask, map, rect, v, jobs);
                out.push(Node::Masked { inner: nodes, keep, off: [rect[0], rect[1]], rect, outside });
            }
            Draw::Projected(p) => self.projected(*p, v, out),
        }
    }

    /// A 2D layer rasterized into its box (target pixels), from the cache when
    /// it hasn't changed.
    fn raster(&mut self, layer: Layer, bounds: Rect, v: &View, jobs: &mut Vec<(Raster, Option<Key>)>) -> Option<(Img, [i32; 2])> {
        let rect = if v.origin.is_some() { v.layer_box(bounds)? } else { v.px(bounds)? };
        let key = Key { id: layer.id, content: crate::canvas::content_key(&layer), rect: v.abs(rect), view: v.key() };
        let off = [rect[0], rect[1]];
        if let Some(e) = self.rasters.get_mut(&key) {
            e.1 = self.frame;
            return Some((Img::Ready(e.0.clone()), off));
        }
        let (full, s) = (v.full, v.s);
        let job: Raster = Box::new(move || {
            let (w, h) = ((rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32);
            let mut pm = Pixmap::new(w, h)?;
            let base = Transform::from_translate(-full.min.x, -full.min.y).post_scale(s, s).post_translate(-rect[0] as f32, -rect[1] as f32);
            let world = Rect::from_min_max(full.min + vec2(rect[0] as f32, rect[1] as f32) / s, full.min + vec2(rect[2] as f32, rect[3] as f32) / s);
            crate::export::draw_layer(&mut pm, &layer, base, world.expand(2.0 / s), s);
            Some((Px::Own(pm.take()), w, h))
        });
        jobs.push((job, Some(key)));
        Some((Img::Job(jobs.len() - 1), off))
    }

    /// A photo or video frame: its pixels (cropped, adjusted, premultiplied, as
    /// `export::image_pixmap`) uploaded once and sampled on the GPU.
    fn image(&mut self, layer: Layer, v: &View, jobs: &mut Vec<(Raster, Option<Key>)>, out: &mut Vec<Node>) {
        let crate::doc::LayerKind::Image(im) = &layer.kind else { return };
        let target = im.scale * v.s;
        // source pixels -> canvas: as export::draw_image (logical size k, and a
        // smaller copy when shown under half size)
        let (iw, ih) = im.src.px_size();
        let (iw, ih) = (iw as f32, ih as f32);
        let k = iw / im.src.size().x.max(1.0);
        let ts = target / k;
        let crop_px = [
            ((im.crop.min.x * k).round()).clamp(0.0, iw - 1.0),
            ((im.crop.min.y * k).round()).clamp(0.0, ih - 1.0),
        ];
        let cw0 = ((im.crop.max.x * k).round()).clamp(crop_px[0] + 1.0, iw) - crop_px[0];
        let ch0 = ((im.crop.max.y * k).round()).clamp(crop_px[1] + 1.0, ih) - crop_px[1];
        let (sw, sh, factor) = if ts < 0.5 {
            let nw = ((cw0 * ts * 1.5).round()).max(1.0);
            let nh = ((ch0 * ts * 1.5).round()).max(1.0);
            (nw, nh, (1.0 / k) * (cw0 / nw))
        } else {
            (cw0, ch0, 1.0 / k)
        };
        // source pixel -> target pixel
        let (sn, cs) = im.rotation.sin_cos();
        let (cw, ch) = (sw * factor, sh * factor);
        let place = Transform::from_row(cs * im.scale, sn * im.scale, -sn * im.scale, cs * im.scale, im.center.x, im.center.y).pre_translate(-cw / 2.0, -ch / 2.0).pre_scale(factor, factor);
        let base = Transform::from_translate(-v.full.min.x, -v.full.min.y).post_scale(v.s, v.s);
        let m = place.post_concat(base);
        let Some(inv) = m.invert() else { return };
        let corners = [(0.0, 0.0), (sw, 0.0), (sw, sh), (0.0, sh)].map(|(x, y)| {
            let mut p = [tiny_skia::Point::from_xy(x, y)];
            m.map_points(&mut p);
            Pos2::new(p[0].x, p[0].y)
        });
        let bb = Rect::from_points(&corners);
        let rect = [
            (bb.min.x.floor() as i32 - 1).max(0),
            (bb.min.y.floor() as i32 - 1).max(0),
            (bb.max.x.ceil() as i32 + 1).min(v.tw as i32),
            (bb.max.y.ceil() as i32 + 1).min(v.th as i32),
        ];
        if rect[2] <= rect[0] || rect[3] <= rect[1] {
            return;
        }
        let nearest = (im.scale * v.s / v.k - 1.0).abs() < 1e-4 && im.rotation == 0.0;
        let px_per_src = (m.sx * m.sy - m.kx * m.ky).abs().sqrt();
        let content = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (im.src.id, im.adjust.key(), sw as u32, sh as u32).hash(&mut h);
            for x in [im.crop.min.x, im.crop.min.y, im.crop.max.x, im.crop.max.y] {
                x.to_bits().hash(&mut h);
            }
            h.finish()
        };
        let key = Key { id: layer.id, content, rect: [0; 4], view: [0; 4] };
        let img = match self.rasters.get_mut(&key) {
            Some(e) => {
                e.1 = self.frame;
                Img::Ready(e.0.clone())
            }
            None => {
                let mut one = im.clone();
                one.center = Pos2::ZERO;
                let whole = crop_px == [0.0, 0.0] && cw0 == iw && ch0 == ih && ts >= 0.5 && im.adjust.is_identity();
                let job: Raster = Box::new(move || {
                    if whole {
                        // the usual video frame: no crop, no adjustment, no smaller
                        // copy; premultiplied in parallel strips (opaque pixels as is)
                        if let Some(f) = &one.src.nv12 {
                            return Some((Px::Nv12(f.clone()), f.w, f.h));
                        }
                        let (w, h) = one.src.rgba.dimensions();
                        if one.src.rgba.as_raw().chunks_exact(4).all(|p| p[3] == 255) {
                            return Some((Px::Shared(one.src.rgba.clone()), w, h));
                        }
                        let mut data = one.src.rgba.as_raw().clone();
                        let strip = (data.len() / 4 / 8).max(1) * 4;
                        std::thread::scope(|sc| {
                            for part in data.chunks_mut(strip) {
                                sc.spawn(move || {
                                    for px in part.chunks_exact_mut(4) {
                                        let a = px[3] as u32;
                                        if a < 255 {
                                            for c in &mut px[..3] {
                                                *c = ((*c as u32 * a + 127) / 255) as u8;
                                            }
                                        }
                                    }
                                });
                            }
                        });
                        return Some((Px::Own(data), w, h));
                    }
                    let (pm, _) = crate::export::image_pixmap(&one, ts * k)?;
                    let (w, h) = (pm.width(), pm.height());
                    Some((Px::Own(pm.take()), w, h))
                });
                jobs.push((job, Some(key)));
                Img::Job(jobs.len() - 1)
            }
        };
        let inv = [[inv.sx, inv.kx, inv.tx], [inv.ky, inv.sy, inv.ty]];
        out.push(Node::Image { img, inv, rect, opacity: layer.opacity, nearest, px_per_src });
    }

    /// A brush layer from its tile atlas (canvas): sampled as a photo is, nearest
    /// at 1:1 and bicubic otherwise. None when it would not fit one texture.
    fn atlas(&mut self, layer: &Layer, v: &View) -> Option<Node> {
        let crate::doc::LayerKind::Raster(r) = &layer.kind else { return None };
        use crate::raster::TILE;
        let (mut tx0, mut ty0, mut tx1, mut ty1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &(tx, ty) in r.tiles.keys() {
            tx0 = tx0.min(tx);
            ty0 = ty0.min(ty);
            tx1 = tx1.max(tx);
            ty1 = ty1.max(ty);
        }
        if tx1 < tx0 {
            return None;
        }
        let limit = (self.max_side / TILE as u32) as i32;
        let fits = |a: &Atlas| a.t0.0 <= tx0 && a.t0.1 <= ty0 && a.t0.0 + a.n.0 > tx1 && a.t0.1 + a.n.1 > ty1;
        let frame = self.frame;
        if !self.atlases.get(&layer.id).is_some_and(fits) {
            // a margin of tiles each way, so painting outward rarely makes it again
            let t0 = (tx0 - 2, ty0 - 2);
            let n = (tx1 - tx0 + 5, ty1 - ty0 + 5);
            if n.0 > limit || n.1 > limit {
                return None;
            }
            let tex = self.texture((n.0 * TILE) as u32, (n.1 * TILE) as u32, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
            self.atlases.insert(layer.id, Atlas { tex: Arc::new(tex), t0, n, revs: HashMap::new(), used: frame });
        }
        let a = self.atlases.get_mut(&layer.id).unwrap();
        a.used = frame;
        let blank = vec![0u8; (TILE * TILE * 4) as usize];
        let put = |q: &wgpu::Queue, a: &Atlas, key: (i32, i32), px: &[u8]| {
            q.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &a.tex, mip_level: 0, origin: wgpu::Origin3d { x: ((key.0 - a.t0.0) * TILE) as u32, y: ((key.1 - a.t0.1) * TILE) as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
                px,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some((TILE * 4) as u32), rows_per_image: Some(TILE as u32) },
                wgpu::Extent3d { width: TILE as u32, height: TILE as u32, depth_or_array_layers: 1 },
            );
        };
        // tiles gone: cleared; tiles new or changed: uploaded
        let gone: Vec<(i32, i32)> = a.revs.keys().copied().filter(|k| !r.tiles.contains_key(k)).collect();
        for k in gone {
            put(&self.queue, a, k, &blank);
            a.revs.remove(&k);
        }
        for (k, t) in &r.tiles {
            if a.revs.get(k) != Some(&t.rev) {
                put(&self.queue, a, *k, &t.px);
                a.revs.insert(*k, t.rev);
            }
        }
        // atlas pixel -> target pixel: one canvas unit per atlas pixel
        let o = Pos2::new(r.offset.x + (a.t0.0 * TILE) as f32, r.offset.y + (a.t0.1 * TILE) as f32);
        let s = v.s;
        let (fx, fy) = ((o.x - v.full.min.x) * s, (o.y - v.full.min.y) * s);
        let inv = [[1.0 / s, 0.0, -fx / s], [0.0, 1.0 / s, -fy / s]];
        let (w, h) = ((a.n.0 * TILE) as f32 * s, (a.n.1 * TILE) as f32 * s);
        let rect = [(fx.floor() as i32).max(0), (fy.floor() as i32).max(0), ((fx + w).ceil() as i32).min(v.tw as i32), ((fy + h).ceil() as i32).min(v.th as i32)];
        if rect[2] <= rect[0] || rect[3] <= rect[1] {
            return None;
        }
        let nearest = (s - 1.0).abs() < 1e-4;
        Some(Node::Image { img: Img::Ready(a.tex.clone()), inv, rect, opacity: layer.opacity, nearest, px_per_src: s })
    }

    /// A mask's keep values (1 − hidden) over `rect`.
    fn keep(&mut self, mask: Layer, map: Option<MaskMap>, rect: [i32; 4], v: &View, jobs: &mut Vec<(Raster, Option<Key>)>) -> Img {
        let mut hm = std::collections::hash_map::DefaultHasher::new();
        {
            use std::hash::Hash;
            (crate::canvas::content_key(&mask), mask.opacity.to_bits(), mask.mask_invert).hash(&mut hm);
            if let Some(m) = map {
                for x in [m.last.x, m.last.y, m.c.x, m.c.y, m.th, m.s] {
                    x.to_bits().hash(&mut hm);
                }
            }
        }
        let key = Key { id: mask.id, content: std::hash::Hasher::finish(&hm), rect: v.abs(rect), view: v.key() };
        if let Some(e) = self.rasters.get_mut(&key) {
            e.1 = self.frame;
            return Img::Ready(e.0.clone());
        }
        let (full, s) = (v.full, v.s);
        let job: Raster = Box::new(move || {
            let (w, h) = ((rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32);
            let mut data = vec![0u8; (w * h * 4) as usize];
            for y in 0..h {
                for x in 0..w {
                    let p = full.min + vec2((rect[0] as f32 + x as f32 + 0.5) / s, (rect[1] as f32 + y as f32 + 0.5) / s);
                    let hide = crate::export::mask_hide(&mask, map, p);
                    let a = (255.0 * (1.0 - hide)).round() as u8;
                    let i = ((y * w + x) * 4) as usize;
                    data[i..i + 4].copy_from_slice(&[a, a, a, 255]);
                }
            }
            Some((Px::Own(data), w, h))
        });
        jobs.push((job, Some(key)));
        Img::Job(jobs.len() - 1)
    }

    fn projected(&mut self, p: Projected, v: &View, out: &mut Vec<Node>) {
        let Projected { layer, pl, cam, clip, light } = p;
        let (full, s) = (v.full, v.s as f64);
        let out_m = [[s, 0.0, -full.min.x as f64 * s], [0.0, s, -full.min.y as f64 * s], [0.0, 0.0, 1.0]];
        let h = crate::persp::mul_mm(&out_m, &pl.homography(&cam));
        let res = crate::export::magnification(&h, layer.bounds().unwrap_or(full)).clamp(0.05, 4.0);
        let clip = clip.map(|m| Rect::from_min_max(((m.min - full.min) * v.s).to_pos2(), ((m.max - full.min) * v.s).to_pos2()));
        // (not when it has got much too coarse: past half the scale needed, one
        // new picture is made even while zooming)
        let fl = if v.zooming { self.flat.nearest(&layer, &pl).filter(|f| f.2 >= res * 0.5) } else { None };
        let Some(fl) = fl.or_else(|| self.flat.get(&layer, &pl, res)) else { return };
        let ptr = Arc::as_ptr(&fl) as usize;
        let tex = match self.flats.get_mut(&ptr) {
            Some(e) => {
                e.2 = self.frame;
                e.1.clone()
            }
            None => {
                let t = self.upload(fl.0.data(), fl.0.width(), fl.0.height());
                self.flats.insert(ptr, (fl.clone(), t.clone(), self.frame));
                t
            }
        };
        let (rect, fres) = (fl.1, fl.2);
        let warp = |h: &crate::persp::M3, shade: f32| -> Option<Node> {
            let hi = crate::persp::invert(h)?;
            let q = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].iter().map(|p| crate::persp::apply_h(h, *p)).collect::<Option<Vec<_>>>()?;
            let mut bb = Rect::from_points(&q);
            if let Some(c) = clip {
                bb = bb.intersect(c);
                if !bb.is_positive() {
                    return None;
                }
            }
            let r = [
                (bb.min.x.floor() as i32 - 1).max(0),
                (bb.min.y.floor() as i32 - 1).max(0),
                (bb.max.x.ceil() as i32 + 1).min(v.tw as i32),
                (bb.max.y.ceil() as i32 + 1).min(v.th as i32),
            ];
            if r[2] <= r[0] || r[3] <= r[1] {
                return None;
            }
            let hi = hi.map(|row| row.map(|x| x as f32));
            Some(Node::Warp { img: Img::Ready(tex.clone()), hi, src: [rect.min.x, rect.min.y, fres], rect: r, opacity: layer.opacity, shade })
        };
        if pl.thick <= 0.0 {
            out.extend(warp(&h, 1.0));
            return;
        }
        let to_eye = |w: crate::persp::V3| crate::persp::sub(cam.center, w);
        if let Some(edges) = &fl.3 {
            // sides in canvas units, kept while outline, placement, camera and
            // light stay the same; then moved into target pixels
            let key = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (Arc::as_ptr(edges) as usize).hash(&mut h);
                serde_json::to_string(&pl).unwrap_or_default().hash(&mut h);
                serde_json::to_string(&light).unwrap_or_default().hash(&mut h);
                format!("{cam:?}").hash(&mut h);
                h.finish()
            };
            let frame = self.frame;
            let canvas_sides = match self.sides.get_mut(&key) {
                Some(e) => {
                    e.1 = frame;
                    e.0.clone()
                }
                None => {
                    let to_c = |w: crate::persp::V3| cam.project(w);
                    let made = Arc::new(crate::extrude::sides(edges, &pl, &to_c, &to_eye, &|w| cam.to_cam(w)[2], light.as_ref(), crate::canvas::SIDE_SHADE));
                    self.sides.insert(key, (made.clone(), frame));
                    made
                }
            };
            let sides: Vec<crate::extrude::Side> = canvas_sides
                .iter()
                .map(|sd| crate::extrude::Side { q: sd.q.map(|p| egui::pos2((p.x - full.min.x) * v.s, (p.y - full.min.y) * v.s)), ..*sd })
                .collect();
            let pts: Vec<Pos2> = sides.iter().flat_map(|sd| sd.q).collect();
            if !pts.is_empty() {
                let bb = Rect::from_points(&pts).expand(2.0);
                let r = [(bb.min.x.floor() as i32).max(0), (bb.min.y.floor() as i32).max(0), (bb.max.x.ceil() as i32).min(v.tw as i32), (bb.max.y.ceil() as i32).min(v.th as i32)];
                let boxr = Rect::from_min_max(egui::pos2(r[0] as f32, r[1] as f32), egui::pos2(r[2] as f32, r[3] as f32));
                let sc = clip.map_or(boxr, |c| c.intersect(boxr));
                if r[2] > r[0] && r[3] > r[1] && sc.is_positive() {
                    let (ox, oy) = (r[0] as f32, r[1] as f32);
                    let (bw, bh) = ((r[2] - r[0]) as u32, (r[3] - r[1]) as u32);
                    let sx0 = ((sc.min.x.floor() - ox).max(0.0) as u32).min(bw - 1);
                    let sy0 = ((sc.min.y.floor() - oy).max(0.0) as u32).min(bh - 1);
                    let sx1 = ((sc.max.x.ceil() - ox).max(0.0) as u32).min(bw);
                    let sy1 = ((sc.max.y.ceil() - oy).max(0.0) as u32).min(bh);
                    let scissor = [sx0, sy0, sx1.saturating_sub(sx0).max(1), sy1.saturating_sub(sy0).max(1)];
                    let alpha = (layer.opacity.clamp(0.0, 1.0) * 255.0).round() / 255.0;
                    let col = |c: [f32; 3]| {
                        let c8 = c.map(|v| (v * 255.0).round().clamp(0.0, 255.0) / 255.0);
                        [c8[0] * alpha, c8[1] * alpha, c8[2] * alpha, alpha]
                    };
                    let mut verts = Vec::with_capacity(sides.len() * 6);
                    for sd in &sides {
                        // as fill_sides: each quad stretched a little along its edge
                        let [a, b, c, d] = sd.q.map(|p| egui::pos2(p.x - ox, p.y - oy));
                        let grow = |p: Pos2, q: Pos2| {
                            let w = q - p;
                            let l = w.length();
                            if l > 1e-3 { p - w / l * 0.35 } else { p }
                        };
                        let (a, d) = (grow(a, b), grow(d, c));
                        let (b, c) = (grow(b, a), grow(c, d));
                        let (ca, cb) = (col(sd.ca), col(sd.cb));
                        let vx = |p: Pos2, c: [f32; 4]| [p.x, p.y, c[0], c[1], c[2], c[3]];
                        verts.extend([vx(a, ca), vx(b, cb), vx(c, cb), vx(a, ca), vx(c, cb), vx(d, ca)]);
                    }
                    out.push(Node::Sides { verts, rect: r, scissor });
                }
            }
        }
        let (off, n) = crate::extrude::visible_cap(&pl, &to_eye);
        let hc = if off == 0.0 { h } else { crate::persp::mul_mm(&out_m, &pl.homography_at(&cam, off)) };
        out.extend(warp(&hc, light.map_or(1.0, |l| l.shade(n))));
    }

    fn clear_to(&self, enc: &mut wgpu::CommandEncoder, t: &wgpu::Texture, c: wgpu::Color) {
        let view = t.create_view(&Default::default());
        let _ = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(c), store: wgpu::StoreOp::Store } })],
            ..Default::default()
        });
    }

    /// One rectangle drawn with `pipe` onto `target`.
    fn pass(&self, enc: &mut wgpu::CommandEncoder, pipe: &wgpu::RenderPipeline, target: &wgpu::Texture, u: &Uniform, t0: &wgpu::Texture, t1: &wgpu::Texture) {
        let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: u.bytes(), usage: wgpu::BufferUsages::UNIFORM });
        let (v0, v1) = (t0.create_view(&Default::default()), t1.create_view(&Default::default()));
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&v0) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&v1) },
            ],
        });
        let view = target.create_view(&Default::default());
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            ..Default::default()
        });
        rp.set_pipeline(pipe);
        rp.set_bind_group(0, &bg, &[]);
        rp.draw(0..6, 0..1);
    }

    fn img(i: &Img, ready: &[Arc<wgpu::Texture>]) -> wgpu::Texture {
        match i {
            Img::Ready(t) => (**t).clone(),
            Img::Job(j) => (*ready[*j]).clone(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run(&mut self, enc: &mut wgpu::CommandEncoder, nd: &Node, target: &wgpu::Texture, depth: usize, ready: &[Arc<wgpu::Texture>], v: &View, bgp: Option<[f32; 4]>) {
        let (tw, th) = (v.tw, v.th);
        match nd {
            Node::Over { img, off, opacity } => {
                let t = Self::img(img, ready);
                let r = [off[0] as f32, off[1] as f32, (off[0] + t.width() as i32) as f32, (off[1] + t.height() as i32) as f32];
                let mut u = Uniform::new(clamp_rect(r, tw, th), tw, th);
                u.tsize[2] = *opacity;
                u.p0 = [off[0] as f32, off[1] as f32, 0.0, 0.0];
                self.pass(enc, &self.over, target, &u, &t, &self.dummy);
            }
            Node::Image { img, inv, rect, opacity, nearest, px_per_src } => {
                let t = Self::img(img, ready);
                let mut u = Uniform::new(rect.map(|x| x as f32), tw, th);
                u.tsize[2] = *opacity;
                u.h0 = [inv[0][0], inv[0][1], inv[0][2], 0.0];
                u.h1 = [inv[1][0], inv[1][1], inv[1][2], 0.0];
                u.p1 = [*px_per_src, *nearest as u8 as f32, 0.0, 0.0];
                self.pass(enc, &self.image, target, &u, &t, &self.dummy);
            }
            Node::Warp { img, hi, src, rect, opacity, shade } => {
                let t = Self::img(img, ready);
                let mut u = Uniform::new(rect.map(|x| x as f32), tw, th);
                u.tsize[2] = *opacity;
                u.tsize[3] = *shade;
                u.h0 = [hi[0][0], hi[0][1], hi[0][2], 0.0];
                u.h1 = [hi[1][0], hi[1][1], hi[1][2], 0.0];
                u.h2 = [hi[2][0], hi[2][1], hi[2][2], 0.0];
                u.p1 = [src[0], src[1], src[2], 0.0];
                self.pass(enc, &self.warp, target, &u, &t, &self.dummy);
            }
            Node::Blend { img, off, mode, opacity } => {
                let t = Self::img(img, ready);
                let r = [off[0] as f32, off[1] as f32, (off[0] + t.width() as i32) as f32, (off[1] + t.height() as i32) as f32];
                let back = self.backdrop(enc, target, depth, tw, th);
                let mut u = Uniform::new(clamp_rect(r, tw, th), tw, th);
                u.tsize[2] = *opacity;
                u.p0 = [off[0] as f32, off[1] as f32, mode_code(*mode), 0.0];
                u.p1[0] = bgp.is_some() as u8 as f32;
                u.bg = bgp.unwrap_or_default();
                self.pass(enc, &self.blend, target, &u, &t, &back);
            }
            Node::Group { inner, rect, mode, opacity } => {
                let g = self.pooled(tw, th, 2 + depth * 2);
                self.clear_to(enc, &g, wgpu::Color::TRANSPARENT);
                for n in inner {
                    self.run(enc, n, &g, depth + 1, ready, v, bgp);
                }
                let back = self.backdrop(enc, target, depth, tw, th);
                let mut u = Uniform::new(rect.map(|x| x as f32), tw, th);
                u.tsize[2] = *opacity;
                u.p0 = [0.0, 0.0, mode_code(*mode), 0.0];
                u.p1[0] = bgp.is_some() as u8 as f32;
                u.bg = bgp.unwrap_or_default();
                self.pass(enc, &self.blend, target, &u, &g, &back);
            }
            Node::Sides { verts, rect, scissor } => {
                let (w, h) = ((rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32);
                let ms = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 4,
                    dimension: wgpu::TextureDimension::D2,
                    format: FMT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                let res = self.texture(w, h, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
                let flat: Vec<f32> = verts.iter().flatten().copied().collect();
                let vb = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: f32_bytes(&flat), usage: wgpu::BufferUsages::VERTEX });
                let u = Uniform::new([0.0, 0.0, w as f32, h as f32], w, h);
                let ub = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: u.bytes(), usage: wgpu::BufferUsages::UNIFORM });
                let dv = self.dummy.create_view(&Default::default());
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&dv) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&dv) },
                    ],
                });
                {
                    let (msv, rv) = (ms.create_view(&Default::default()), res.create_view(&Default::default()));
                    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &msv,
                            depth_slice: None,
                            resolve_target: Some(&rv),
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard },
                        })],
                        ..Default::default()
                    });
                    rp.set_pipeline(&self.tri);
                    rp.set_bind_group(0, &bg, &[]);
                    rp.set_vertex_buffer(0, vb.slice(..));
                    rp.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
                    rp.draw(0..verts.len() as u32, 0..1);
                }
                let r = [rect[0] as f32, rect[1] as f32, rect[2] as f32, rect[3] as f32];
                let mut u = Uniform::new(r, tw, th);
                u.p0 = [rect[0] as f32, rect[1] as f32, 0.0, 0.0];
                self.pass(enc, &self.over, target, &u, &res, &self.dummy);
            }
            Node::Masked { inner, keep, off, rect, outside } => {
                let g = self.pooled(tw, th, 2 + depth * 2);
                self.clear_to(enc, &g, wgpu::Color::TRANSPARENT);
                for n in inner {
                    self.run(enc, n, &g, depth + 1, ready, v, bgp);
                }
                let k = Self::img(keep, ready);
                let mut u = Uniform::new(rect.map(|x| x as f32), tw, th);
                u.p0 = [off[0] as f32, off[1] as f32, 0.0, *outside];
                self.pass(enc, &self.mask, target, &u, &g, &k);
            }
        }
    }

    /// A copy of `target` as it is now (blend modes read what is below).
    fn backdrop(&mut self, enc: &mut wgpu::CommandEncoder, target: &wgpu::Texture, depth: usize, tw: u32, th: u32) -> wgpu::Texture {
        let b = self.pooled(tw, th, 3 + depth * 2);
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &b, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: tw, height: th, depth_or_array_layers: 1 },
        );
        b
    }
}

/// A brush layer's tiles in one texture: tile `t0` at its top left, `n` tiles
/// each way, and the revision of each tile uploaded.
struct Atlas {
    tex: Arc<wgpu::Texture>,
    t0: (i32, i32),
    n: (i32, i32),
    revs: HashMap<(i32, i32), u64>,
    used: u64,
}

/// The canvas ground (theme::draw_ground): void colour, two soft blooms and the
/// drifting dot grid. Colours premultiplied, 0..1; positions in target pixels.
pub struct Ground {
    pub void: [f32; 4],
    pub warm: [f32; 4],
    pub cool: [f32; 4],
    /// Centre x, y and radius of each bloom.
    pub warm_at: [f32; 4],
    pub cool_at: [f32; 4],
    pub dot: [f32; 4],
    /// Dot spacing in canvas units, and radius in target pixels.
    pub step: f32,
    pub dot_r: f32,
}

/// The drawn picture: its canvas rect, target pixels per unit and size, and the
/// supersampling factor.
#[derive(Clone, Copy)]
struct View {
    full: Rect,
    s: f32,
    tw: u32,
    th: u32,
    k: f32,
    /// Canvas mode: `full.min · s` in whole pixels. Layer pictures are then kept
    /// by their place in canvas pixels, so panning reuses them.
    origin: Option<[i32; 2]>,
    /// The canvas is zooming: layers in 3D spaces reuse a picture already made.
    zooming: bool,
}

impl View {
    /// A canvas rect in whole target pixels (a pixel of margin), None when off.
    fn px(&self, r: Rect) -> Option<[i32; 4]> {
        let a = (r.min - self.full.min) * self.s;
        let b = (r.max - self.full.min) * self.s;
        let p = [
            (a.x.floor() as i32 - 1).max(0),
            (a.y.floor() as i32 - 1).max(0),
            (b.x.ceil() as i32 + 1).min(self.tw as i32),
            (b.y.ceil() as i32 + 1).min(self.th as i32),
        ];
        (p[2] > p[0] && p[3] > p[1]).then_some(p)
    }

    fn key(&self) -> [u32; 4] {
        match self.origin {
            Some(_) => [0, 0, self.s.to_bits(), self.k.to_bits()],
            None => [self.full.min.x.to_bits(), self.full.min.y.to_bits(), self.s.to_bits(), self.k.to_bits()],
        }
    }

    /// A target rect in canvas pixels (canvas mode), for cache keys.
    fn abs(&self, r: [i32; 4]) -> [i32; 4] {
        match self.origin {
            Some([ox, oy]) => [r[0] + ox, r[1] + oy, r[2] + ox, r[3] + oy],
            None => r,
        }
    }

    /// Canvas mode: a layer's box in target pixels, the whole layer when it is
    /// not huge (so panning keeps its picture), else what is in view and a
    /// margin, on a 512 px grid of canvas pixels.
    fn layer_box(&self, b: Rect) -> Option<[i32; 4]> {
        let [ox, oy] = self.origin?;
        let a = [(b.min.x * self.s).floor() as i32 - 1, (b.min.y * self.s).floor() as i32 - 1, (b.max.x * self.s).ceil() as i32 + 1, (b.max.y * self.s).ceil() as i32 + 1];
        if a[2] <= a[0] || a[3] <= a[1] {
            return None;
        }
        let mut r = a;
        if a[2] - a[0] > 4096 || a[3] - a[1] > 4096 {
            let g = 512;
            let v = [ox - g, oy - g, ox + self.tw as i32 + g, oy + self.th as i32 + g];
            let v = [v[0].div_euclid(g) * g, v[1].div_euclid(g) * g, (v[2] + g - 1).div_euclid(g) * g, (v[3] + g - 1).div_euclid(g) * g];
            r = [a[0].max(v[0]), a[1].max(v[1]), a[2].min(v[2]), a[3].min(v[3])];
        }
        // visible at all?
        if r[2] <= ox || r[3] <= oy || r[0] >= ox + self.tw as i32 || r[1] >= oy + self.th as i32 || r[2] <= r[0] || r[3] <= r[1] {
            return None;
        }
        Some([r[0] - ox, r[1] - oy, r[2] - ox, r[3] - oy])
    }
}

fn f32_bytes(v: &[f32]) -> &[u8] {
    // plain f32s
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

fn clamp_rect(r: [f32; 4], tw: u32, th: u32) -> [f32; 4] {
    [r[0].max(0.0), r[1].max(0.0), r[2].min(tw as f32), r[3].min(th as f32)]
}

fn mode_code(m: Blend) -> f32 {
    Blend::ALL.iter().position(|b| *b == m).unwrap_or(0) as f32
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Uniform {
    rect: [f32; 4],
    tsize: [f32; 4],
    h0: [f32; 4],
    h1: [f32; 4],
    h2: [f32; 4],
    p0: [f32; 4],
    p1: [f32; 4],
    bg: [f32; 4],
    rot: [f32; 4],
    rot2: [f32; 4],
}

impl Uniform {
    fn new(rect: [f32; 4], tw: u32, th: u32) -> Self {
        Uniform { rect, tsize: [tw as f32, th as f32, 1.0, 1.0], rot: [1.0, 0.0, 0.0, 0.0], ..Default::default() }
    }

    fn bytes(&self) -> &[u8] {
        // plain f32 arrays, no padding
        unsafe { std::slice::from_raw_parts(self as *const Self as *const u8, std::mem::size_of::<Self>()) }
    }
}
