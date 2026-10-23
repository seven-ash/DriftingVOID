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
    /// A photo or video frame through `inv` (target pixel -> source pixel),
    /// sampled between the mip levels about `level`.
    Image { img: Img, inv: [[f32; 3]; 2], rect: [i32; 4], opacity: f32, nearest: bool, px_per_src: f32, level: f32 },
    /// A flattened layer through `hi` (inverse homography, target -> canvas).
    Warp { img: Img, hi: [[f32; 3]; 3], src: [f32; 3], rect: [i32; 4], opacity: f32, shade: f32 },
    /// A photo or video frame in a 3D space through `hi` (inverse homography,
    /// target -> source pixel), from its mipmapped texture.
    WarpImage { tex: Arc<wgpu::Texture>, hi: [[f32; 3]; 3], rect: [i32; 4], opacity: f32 },
    /// Text with a blend mode.
    Blend { img: Img, off: [i32; 2], mode: Blend, opacity: f32 },
    /// Drawn alone into a target of its own, then blended down once.
    Group { inner: Vec<Node>, rect: [i32; 4], mode: Blend, opacity: f32 },
    /// Extrusion sides built on the GPU from the outline's edges (`sides.wgsl`,
    /// one instance per edge) into `rect`'s box, multisampled with a depth
    /// buffer, then put over the target; `scissor` in box pixels.
    Sides { edges: Arc<wgpu::Buffer>, count: u32, u: SideUniform, rect: [i32; 4], scissor: [u32; 4] },
    /// Drawn alone, then multiplied by the alpha of `opening` (drawn alone
    /// too): an engraving's floor and inner walls seen through its shape.
    Cut { inner: Vec<Node>, opening: Vec<Node>, rect: [i32; 4] },
    /// Drawn alone, then multiplied by a mask's keep values.
    Masked { inner: Vec<Node>, keep: Img, off: [i32; 2], rect: [i32; 4], outside: f32 },
    /// Drawn alone, then cut by a mask sampled from its tile atlas: `a` maps a
    /// target pixel to an atlas pixel (the mask's own pixels; nearest, as
    /// export::mask_hide), `invert` and `strength` as the mask layer's.
    MaskedGpu { inner: Vec<Node>, atlas: Arc<wgpu::Texture>, a: [[f32; 3]; 2], rect: [i32; 4], invert: bool, strength: f32 },
}

/// A texture: cached, or still to be rasterized (index into the job list).
enum Img {
    Ready(Arc<wgpu::Texture>),
    Job(usize),
}

type Raster = Box<dyn FnOnce() -> Option<(Vec<u8>, u32, u32)> + Send>;

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
    warp_image: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    mask: wgpu::RenderPipeline,
    cut: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    sides_layout: wgpu::BindGroupLayout,
    sides_pipe: wgpu::RenderPipeline,
    path_layout: wgpu::BindGroupLayout,
    path_pipe: wgpu::RenderPipeline,
    /// Pen strokes (`strokes.wgsl`): capsules into a coverage target, then
    /// the colour over the layer, per stroke.
    stroke_layout: wgpu::BindGroupLayout,
    stroke_seg: wgpu::RenderPipeline,
    stroke_zero: wgpu::RenderPipeline,
    stroke_put: wgpu::RenderPipeline,
    image: wgpu::RenderPipeline,
    y_plane: wgpu::RenderPipeline,
    uv_plane: wgpu::RenderPipeline,
    from_nv12: wgpu::RenderPipeline,
    ground: wgpu::RenderPipeline,
    mask_atlas: wgpu::RenderPipeline,
    prep: wgpu::RenderPipeline,
    mip: wgpu::RenderPipeline,
    dummy: wgpu::Texture,
    pub max_side: u32,
    pub adapter: String,
    /// PCI vendor id (0x10de: NVIDIA).
    pub vendor: u32,
    /// Flattened 3D layers, kept across a job's frames.
    flat: Arc<FlatCache>,
    /// Rasterized 2D layers and mask keep images, by content and placement;
    /// photos and video frames (cropped, adjusted, with mipmaps) by content.
    rasters: HashMap<Key, (Arc<wgpu::Texture>, u64)>,
    /// Photos and video frames as they came (straight alpha), by source id,
    /// while their prepared copy is made again (an adjustment being dragged).
    sources: HashMap<u64, (Arc<wgpu::Texture>, u64)>,
    /// Textures of flattened 3D layers (the entry is kept so its address stays unique).
    flats: HashMap<usize, (Arc<(Pixmap, Rect, f32, Option<Arc<Vec<crate::extrude::Edge>>>)>, Arc<wgpu::Texture>, u64)>,
    /// Outlines of 3D layers uploaded as edge instances (`sides.wgsl`), by the
    /// outline (kept so its address stays unique).
    edge_bufs: HashMap<usize, (Arc<Vec<crate::extrude::Edge>>, Arc<wgpu::Buffer>, u64)>,
    /// Full-size targets: (w, h, slot).
    pool: HashMap<(u32, u32, usize), wgpu::Texture>,
    /// Sides targets by size (multisampled colour, depth, resolved colour).
    ms_pool: HashMap<(u32, u32), (wgpu::Texture, wgpu::Texture, wgpu::Texture, u64)>,
    /// Brush layers on the canvas: their tiles in one texture each, updated tile
    /// by tile as they change (painting re-uploads only the touched tiles).
    atlases: HashMap<LayerId, Atlas>,
    /// Brush layers on the canvas: their tiles in pages of PAGE×PAGE tiles,
    /// only where there are tiles (any size of layer; no texture for the
    /// empty space between strokes).
    pages: HashMap<LayerId, Paged>,
    frame: u64,
    /// A picture drawn between the canvas's frames (`render_aside`).
    aside: bool,
    /// Glyph outlines (font units), and their copy on the GPU (by `rev`).
    glyphs: crate::gpu_paths::Glyphs,
    glyph_bufs: Option<(u64, [Arc<wgpu::Buffer>; 3])>,
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

/// A photo's crop in source pixels: logical-to-pixel factor `k`, top left
/// `px`, size `w`×`h`, and a key for its look (source, adjustment, crop).
struct Crop {
    k: f32,
    px: [f32; 2],
    w: f32,
    h: f32,
    key: u64,
}

fn crop_of(im: &crate::doc::ImageLayer) -> Crop {
    // source pixels -> canvas: as export::draw_image (logical size k)
    let (iw, ih) = im.src.px_size();
    let (iw, ih) = (iw as f32, ih as f32);
    let k = iw / im.src.size().x.max(1.0);
    let px = [((im.crop.min.x * k).round()).clamp(0.0, iw - 1.0), ((im.crop.min.y * k).round()).clamp(0.0, ih - 1.0)];
    let w = ((im.crop.max.x * k).round()).clamp(px[0] + 1.0, iw) - px[0];
    let h = ((im.crop.max.y * k).round()).clamp(px[1] + 1.0, ih) - px[1];
    // the source as it is shown, whatever size it is drawn at
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        (im.src.id, im.adjust.key()).hash(&mut hs);
        for x in [im.crop.min.x, im.crop.min.y, im.crop.max.x, im.crop.max.y] {
            x.to_bits().hash(&mut hs);
        }
        hs.finish()
    };
    Crop { k, px, w, h, key }
}

/// The graphics APIs to use: DX12 on Windows (Vulkan's vsync there presents
/// every other refresh in a window: 82.5 fps on a 165 Hz screen, measured
/// 10-22), the platform's own elsewhere; `WGPU_BACKEND` overrides.
pub fn backends() -> wgpu::Backends {
    wgpu::Backends::from_env().unwrap_or(if cfg!(windows) { wgpu::Backends::DX12 } else { wgpu::Backends::PRIMARY })
}

/// Called once at start when eframe runs on wgpu.
pub fn install(rs: &eframe::egui_wgpu::RenderState) {
    let info = rs.adapter.get_info();
    if std::env::var_os("DV_TIMING").is_some() {
        eprintln!("window GPU: {} ({:?})", info.name, info.backend);
    }
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
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = backends();
        let instance = wgpu::Instance::new(desc);
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
        let mask_atlas = pipe("fs_mask_atlas", Some(over_blend));
        let over = pipe("fs_over", Some(over_blend));
        let warp = pipe("fs_warp", Some(over_blend));
        let warp_image = pipe("fs_warp_image", Some(over_blend));
        let blend = pipe("fs_blend", None);
        let mask = pipe("fs_mask", Some(over_blend));
        let cut = pipe("fs_cut", Some(over_blend));
        let down = pipe("fs_down", None);
        let image = pipe("fs_image", Some(over_blend));
        let prep = pipe("fs_prep", None);
        let mip = pipe("fs_mip", None);
        let sides_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sides"), source: wgpu::ShaderSource::Wgsl(include_str!("sides.wgsl").into()) });
        let sides_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let sides_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&sides_layout)], immediate_size: 0 });
        let sides_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sides"),
            layout: Some(&sides_pl),
            vertex: wgpu::VertexState {
                module: &sides_module,
                entry_point: Some("vs_side"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: EDGE_BYTES,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x2, 5 => Unorm8x4, 6 => Unorm8x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &sides_module,
                entry_point: Some("fs_side"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: FMT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: 4, ..Default::default() },
            multiview_mask: None,
            cache: None,
        });
        let path_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("paths"), source: wgpu::ShaderSource::Wgsl(include_str!("paths.wgsl").into()) });
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let path_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                storage(1),
                storage(2),
                storage(3),
            ],
        });
        let path_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&path_layout)], immediate_size: 0 });
        let path_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("paths"),
            layout: Some(&path_pl),
            vertex: wgpu::VertexState {
                module: &path_module,
                entry_point: Some("vs_path"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: crate::gpu_paths::INST_BYTES,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Uint32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &path_module,
                entry_point: Some("fs_path"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: FMT, blend: Some(over_blend), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let stroke_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("strokes"), source: wgpu::ShaderSource::Wgsl(include_str!("strokes.wgsl").into()) });
        let stroke_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None },
                    count: None,
                },
                tex_entry(1),
            ],
        });
        let stroke_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&stroke_layout)], immediate_size: 0 });
        let max_blend = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Max },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Max },
        };
        let stroke_pipe = |vs: &str, fs: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>, seg: bool| {
            let attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];
            let buffers = [Some(wgpu::VertexBufferLayout { array_stride: 32, step_mode: wgpu::VertexStepMode::Instance, attributes: &attrs })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&stroke_pl),
                vertex: wgpu::VertexState { module: &stroke_module, entry_point: Some(vs), compilation_options: Default::default(), buffers: if seg { &buffers } else { &[] } },
                fragment: Some(wgpu::FragmentState {
                    module: &stroke_module,
                    entry_point: Some(fs),
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
        let stroke_seg = stroke_pipe("vs_seg", "fs_seg", wgpu::TextureFormat::R8Unorm, Some(max_blend), true);
        let stroke_zero = stroke_pipe("vs_rect", "fs_zero", wgpu::TextureFormat::R8Unorm, None, false);
        let stroke_put = stroke_pipe("vs_rect", "fs_put", FMT, Some(over_blend), false);
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
            warp_image,
            blend,
            mask,
            cut,
            down,
            sides_layout,
            sides_pipe,
            path_layout,
            path_pipe,
            stroke_layout,
            stroke_seg,
            stroke_zero,
            stroke_put,
            image,
            y_plane,
            uv_plane,
            from_nv12,
            ground,
            mask_atlas,
            prep,
            mip,
            dummy,
            max_side,
            adapter,
            vendor,
            flat: Arc::default(),
            rasters: HashMap::new(),
            sources: HashMap::new(),
            flats: HashMap::new(),
            pool: HashMap::new(),
            ms_pool: HashMap::new(),
            atlases: HashMap::new(),
            pages: HashMap::new(),
            edge_bufs: HashMap::new(),
            frame: 0,
            aside: false,
            glyphs: Default::default(),
            glyph_bufs: None,
        }
    }

    /// Drops everything kept between frames (after a job).
    pub fn clear(&mut self) {
        self.atlases.clear();
        self.pages.clear();
        self.edge_bufs.clear();
        self.flat = Arc::default();
        self.rasters.clear();
        self.sources.clear();
        self.flats.clear();
        self.pool.clear();
        self.ms_pool.clear();
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

    /// As `export::render`, on the canvas's renderer between its frames (the
    /// loupe, the eyedropper, copying, the auto-selection tools' input): what
    /// the canvas keeps is neither dropped nor aged, so its photos, masks and
    /// 3D layers are reused, and the targets made for this picture are let go.
    pub fn render_aside(&mut self, doc: &Document, region: Rect, scale: f32, bg: Option<Color32>) -> Result<Pixmap, String> {
        let had: std::collections::HashSet<(u32, u32, usize)> = self.pool.keys().copied().collect();
        self.aside = true;
        let p = self.submit(doc, region.center(), region.size(), 0.0, scale, bg, 1, false);
        self.aside = false;
        self.pool.retain(|k, _| had.contains(k));
        match self.finish(p?)? {
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
        if !self.aside {
            self.frame += 1;
            let g = self.frame;
            self.rasters.retain(|_, v| v.1 + 1 >= g);
            self.sources.retain(|_, v| v.1 + 1 >= g);
            self.flats.retain(|_, v| v.2 + 1 >= g);
            self.edge_bufs.retain(|_, v| v.2 + 1 >= g);
            self.ms_pool.retain(|_, v| v.3 + 1 >= g);
            self.flat.sweep();
        }

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
    /// target's top left, `s` pixels per canvas unit) drawn over the ground (or
    /// over nothing) into `out` (Rgba8Unorm, the window's pixels). Nothing is
    /// read back; egui shows `out`. Returns the sub-pixel offset of the drawing
    /// (target pixels) to place it at, since the drawing snaps to whole canvas
    /// pixels. `view`: a 3D workspace camera (its "canvas" is the screen).
    #[allow(clippy::too_many_arguments)]
    pub fn render_canvas(&mut self, doc: &Document, world_min: Pos2, s: f32, out: &wgpu::Texture, ground: Option<&Ground>, zooming: bool, view: Option<&crate::scene::ViewCam>) -> egui::Vec2 {
        let (tw, th) = (out.width(), out.height());
        let ox = (world_min.x * s).floor();
        let oy = (world_min.y * s).floor();
        let frac = vec2(world_min.x * s - ox, world_min.y * s - oy);
        let full = Rect::from_min_size(Pos2::new(ox / s, oy / s), vec2(tw as f32 / s, th as f32 / s));
        self.frame += 1;
        let g = self.frame;
        // the canvas keeps pictures a little longer (a few frames of panning);
        // 3D layers' flattened pictures, outlines and sides much longer (they
        // are dear to make again when a layer comes back into view)
        self.flat.set_keep(1800);
        self.rasters.retain(|_, v| v.1 + 30 >= g);
        self.sources.retain(|_, v| v.1 + 2 >= g);
        self.flats.retain(|_, v| v.2 + 1800 >= g);
        self.atlases.retain(|_, a| a.used + 600 >= g);
        self.pages.retain(|_, p| p.used + 600 >= g);
        self.edge_bufs.retain(|_, v| v.2 + 1800 >= g);
        self.ms_pool.retain(|_, v| v.3 + 30 >= g);
        self.flat.sweep();
        let t0 = std::time::Instant::now();
        let scene = crate::scene::build_view(doc, view);
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
        match ground {
            Some(ground) => {
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
            }
            None => self.clear_to(&mut enc, out, wgpu::Color::TRANSPARENT),
        }
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
        let done: Mutex<Vec<Option<(Vec<u8>, u32, u32)>>> = Mutex::new((0..n).map(|_| None).collect());
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
                self.atlas(&layer, v, out);
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
                // on the GPU from the mask's tiles (nothing to compute per pixel
                // on the CPU, so zooming costs nothing); the CPU keep values
                // only when the mask is too big for one texture
                if let Some((atlas, o)) = self.atlas_of(&mask) {
                    // target pixel -> canvas -> (the mask's own place) -> atlas pixel
                    let sv = v.s;
                    let f = v.full.min;
                    let (r, sc, c, last) = match map {
                        Some(m) => (m.th, m.s.max(1e-6), m.c, m.last),
                        None => (0.0, 1.0, Pos2::ZERO, Pos2::ZERO),
                    };
                    let (sn, cs) = (-r).sin_cos();
                    // q = last + R(-th)·((F + P/sv) − c)/s − O
                    let k = 1.0 / (sv * sc);
                    let (dx, dy) = ((f.x - c.x) / sc, (f.y - c.y) / sc);
                    let a = [
                        [cs * k, -sn * k, last.x + cs * dx - sn * dy - o.x],
                        [sn * k, cs * k, last.y + sn * dx + cs * dy - o.y],
                    ];
                    out.push(Node::MaskedGpu { inner: nodes, atlas, a, rect, invert: mask.mask_invert, strength });
                    return;
                }
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
        if crate::gpu_paths::handles(&layer) && std::env::var_os("DV_CPU_2D").is_none() {
            // text and shapes: filled on the GPU from their outlines
            let t = self.raster_paths(&layer, rect, v)?;
            self.rasters.insert(key, (t.clone(), self.frame));
            return Some((Img::Ready(t), off));
        }
        let (full, s) = (v.full, v.s);
        let job: Raster = Box::new(move || {
            let (w, h) = ((rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32);
            let mut pm = Pixmap::new(w, h)?;
            let base = Transform::from_translate(-full.min.x, -full.min.y).post_scale(s, s).post_translate(-rect[0] as f32, -rect[1] as f32);
            let world = Rect::from_min_max(full.min + vec2(rect[0] as f32, rect[1] as f32) / s, full.min + vec2(rect[2] as f32, rect[3] as f32) / s);
            crate::export::draw_layer(&mut pm, &layer, base, world.expand(2.0 / s), s);
            Some((pm.take(), w, h))
        });
        jobs.push((job, Some(key)));
        Some((Img::Job(jobs.len() - 1), off))
    }

    /// Text or a shape drawn into its box (target pixels `rect`) on the GPU:
    /// glyphs from the kept outlines, shape outlines made for this drawing.
    fn raster_paths(&mut self, layer: &Layer, rect: [i32; 4], v: &View) -> Option<Arc<wgpu::Texture>> {
        use crate::gpu_paths::inst_bytes;
        if matches!(layer.kind, crate::doc::LayerKind::Vector(_)) {
            return self.raster_strokes(layer, rect, v);
        }
        let (w, h) = ((rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32);
        let (full, s) = (v.full, v.s);
        let base = Transform::from_translate(-full.min.x, -full.min.y).post_scale(s, s).post_translate(-rect[0] as f32, -rect[1] as f32);
        let d = crate::gpu_paths::drawing(&mut self.glyphs, layer, base, s)?;
        let tex = self.texture(w, h, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let mut u = Uniform::new([0.0; 4], w, h);
        u.tsize = [w as f32, h as f32, 0.0, 0.0];
        let ub = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &u.bytes()[16..32], usage: wgpu::BufferUsages::UNIFORM });
        let storage = |dev: &wgpu::Device, bytes: &[u8]| {
            // never empty (a binding needs a size)
            let pad = [0u8; 16];
            let bytes = if bytes.is_empty() { &pad[..] } else { bytes };
            Arc::new(dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytes, usage: wgpu::BufferUsages::STORAGE }))
        };
        let set_bufs = |dev: &wgpu::Device, set: &crate::gpu_paths::PathSet| -> [Arc<wgpu::Buffer>; 3] {
            [storage(dev, f32_bytes(set.edges.as_flattened())), storage(dev, u32_bytes(set.bands.as_flattened())), storage(dev, u32_bytes(&set.idx))]
        };
        if !d.glyphs.is_empty() && self.glyph_bufs.as_ref().is_none_or(|g| g.0 != self.glyphs.rev) {
            self.glyph_bufs = Some((self.glyphs.rev, set_bufs(&self.device, &self.glyphs.set)));
        }
        let bind = |dev: &wgpu::Device, layout: &wgpu::BindGroupLayout, b: &[Arc<wgpu::Buffer>; 3]| {
            dev.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: b[0].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: b[1].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: b[2].as_entire_binding() },
                ],
            })
        };
        // a text's background, then its glyphs; a shape's fill, then its stroke
        let mut draws: Vec<(wgpu::BindGroup, wgpu::Buffer, u32)> = Vec::new();
        if !d.shapes.is_empty() {
            let b = set_bufs(&self.device, &d.local);
            let vb = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: inst_bytes(&d.shapes), usage: wgpu::BufferUsages::VERTEX });
            draws.push((bind(&self.device, &self.path_layout, &b), vb, d.shapes.len() as u32));
        }
        if let (false, Some((_, b))) = (d.glyphs.is_empty(), &self.glyph_bufs) {
            let vb = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: inst_bytes(&d.glyphs), usage: wgpu::BufferUsages::VERTEX });
            draws.push((bind(&self.device, &self.path_layout, b), vb, d.glyphs.len() as u32));
        }
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let view = tex.create_view(&Default::default());
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                ..Default::default()
            });
            rp.set_pipeline(&self.path_pipe);
            for (bg, vb, n) in &draws {
                rp.set_bind_group(0, bg, &[]);
                rp.set_vertex_buffer(0, vb.slice(..));
                rp.draw(0..6, 0..*n);
            }
        }
        self.queue.submit([enc.finish()]);
        Some(Arc::new(tex))
    }

    /// A pen layer drawn into its box (target pixels `rect`) on the GPU, stroke
    /// by stroke (`strokes.wgsl`): each stroke's capsules take the largest
    /// coverage per pixel, then its colour goes over what the strokes before
    /// it left, as the CPU fills each stroke's union in turn.
    fn raster_strokes(&mut self, layer: &Layer, rect: [i32; 4], v: &View) -> Option<Arc<wgpu::Texture>> {
        let crate::doc::LayerKind::Vector(vl) = &layer.kind else { return None };
        let (w, h) = ((rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32);
        let (full, s) = (v.full, v.s);
        let px = |x: f32, y: f32| [(x + vl.offset.x - full.min.x) * s - rect[0] as f32, (y + vl.offset.y - full.min.y) * s - rect[1] as f32];
        let boxr = Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32));
        // per stroke: its uniform (256-byte slots) and its capsules
        const SLOT: usize = 256;
        let mut uni: Vec<u8> = Vec::new();
        let mut segs: Vec<f32> = Vec::new();
        let mut list: Vec<(u32, std::ops::Range<u32>, [f32; 4])> = Vec::new();
        for st in &vl.strokes {
            if st.pts.is_empty() {
                continue;
            }
            let (a, b) = (px(st.bounds.min.x, st.bounds.min.y), px(st.bounds.max.x, st.bounds.max.y));
            let r = Rect::from_min_max(egui::pos2(a[0], a[1]), egui::pos2(b[0], b[1])).expand(1.5).intersect(boxr);
            if !r.is_positive() {
                continue;
            }
            let r = [r.min.x.floor(), r.min.y.floor(), r.max.x.ceil(), r.max.y.ceil()];
            let first = (segs.len() / 8) as u32;
            let mut cap = |p: &[f32; 3], q: &[f32; 3]| {
                let (p0, q0) = (px(p[0], p[1]), px(q[0], q[1]));
                segs.extend_from_slice(&[p0[0], p0[1], p[2] * s, 0.0, q0[0], q0[1], q[2] * s, 0.0]);
            };
            if st.pts.len() == 1 {
                cap(&st.pts[0], &st.pts[0]);
            }
            for pair in st.pts.windows(2) {
                cap(&pair[0], &pair[1]);
            }
            let [cr, cg, cb, ca] = st.color.to_srgba_unmultiplied().map(|x| x as f32 / 255.0);
            let off = uni.len() as u32;
            for x in [r[0], r[1], r[2], r[3], w as f32, h as f32, 0.0, 0.0, cr * ca, cg * ca, cb * ca, ca] {
                uni.extend_from_slice(&x.to_le_bytes());
            }
            uni.resize(off as usize + SLOT, 0);
            list.push((off, first..(segs.len() / 8) as u32, r));
        }
        let tex = self.texture(w, h, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.clear_to(&mut enc, &tex, wgpu::Color::TRANSPARENT);
        if !list.is_empty() {
            let cov = self.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let ub = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &uni, usage: wgpu::BufferUsages::UNIFORM });
            let vb = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: f32_bytes(&segs), usage: wgpu::BufferUsages::VERTEX });
            let (cov_view, out_view, dummy_view) = (cov.create_view(&Default::default()), tex.create_view(&Default::default()), self.dummy.create_view(&Default::default()));
            let group = |t: &wgpu::TextureView| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.stroke_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &ub, offset: 0, size: std::num::NonZeroU64::new(48) }) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(t) },
                    ],
                })
            };
            let (g_cov, g_put) = (group(&dummy_view), group(&cov_view));
            let pass = |enc: &'_ mut wgpu::CommandEncoder, view: &wgpu::TextureView| {
                enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                    ..Default::default()
                })
                .forget_lifetime()
            };
            for (off, range, _) in &list {
                {
                    let mut rp = pass(&mut enc, &cov_view);
                    rp.set_bind_group(0, &g_cov, &[*off]);
                    rp.set_pipeline(&self.stroke_zero);
                    rp.draw(0..6, 0..1);
                    rp.set_pipeline(&self.stroke_seg);
                    rp.set_vertex_buffer(0, vb.slice(..));
                    rp.draw(0..6, range.clone());
                }
                let mut rp = pass(&mut enc, &out_view);
                rp.set_bind_group(0, &g_put, &[*off]);
                rp.set_pipeline(&self.stroke_put);
                rp.draw(0..6, 0..1);
            }
        }
        self.queue.submit([enc.finish()]);
        Some(Arc::new(tex))
    }

    /// A photo or video frame: uploaded once as it came, then cropped, adjusted,
    /// premultiplied and given mipmaps on the GPU (`prepared`), and sampled from
    /// the level that suits its size on screen, so zooming makes nothing again.
    /// Only a source too big for one texture goes through the CPU
    /// (`export::image_pixmap`).
    fn image(&mut self, layer: Layer, v: &View, jobs: &mut Vec<(Raster, Option<Key>)>, out: &mut Vec<Node>) {
        let crate::doc::LayerKind::Image(im) = &layer.kind else { return };
        let target = im.scale * v.s;
        let c = crop_of(im);
        let (k, cw0, ch0, src_key) = (c.k, c.w, c.h, c.key);
        let on_gpu = cw0 as u32 <= self.max_side && ch0 as u32 <= self.max_side;
        let (sw, sh, factor, ts) = if on_gpu {
            (cw0, ch0, 1.0 / k, 1.0)
        } else {
            // a smaller copy when shown under half size (on the canvas in
            // halvings only, so zooming out doesn't re-make it every step)
            let mut ts = target / k;
            if v.origin.is_some() && ts < 0.5 {
                let f = 2f32.powf((ts * 1.5).log2().ceil());
                ts = if f >= 1.0 { 0.5 } else { f / 1.5 };
            }
            let ts = ts.max(1e-4);
            if ts < 0.5 {
                let nw = ((cw0 * ts * 1.5).round()).max(1.0);
                let nh = ((ch0 * ts * 1.5).round()).max(1.0);
                (nw, nh, (1.0 / k) * (cw0 / nw), ts)
            } else {
                (cw0, ch0, 1.0 / k, ts)
            }
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
        let inv = [[inv.sx, inv.kx, inv.tx], [inv.ky, inv.sy, inv.ty]];
        if on_gpu {
            let tex = self.image_tex(layer.id, im, &c);
            // shown under half size: between the levels about 1.5 texels per
            // target pixel (as the CPU's smaller copy at 1.5)
            let level = match nearest || px_per_src >= 0.5 {
                true => 0.0,
                false => (1.0 / (1.5 * px_per_src)).log2().clamp(0.0, (tex.mip_level_count() - 1) as f32),
            };
            out.push(Node::Image { img: Img::Ready(tex), inv, rect, opacity: layer.opacity, nearest, px_per_src, level });
            return;
        }
        let key = Key { id: layer.id, content: src_key, rect: [sw as i32, sh as i32, 0, 0], view: [0; 4] };
        let img = match self.rasters.get_mut(&key) {
            Some(e) => {
                e.1 = self.frame;
                Img::Ready(e.0.clone())
            }
            None => {
                let mut one = im.clone();
                one.center = Pos2::ZERO;
                let job: Raster = Box::new(move || {
                    let (pm, _) = crate::export::image_pixmap(&one, ts * k)?;
                    let (w, h) = (pm.width(), pm.height());
                    Some((pm.take(), w, h))
                });
                jobs.push((job, Some(key)));
                Img::Job(jobs.len() - 1)
            }
        };
        out.push(Node::Image { img, inv, rect, opacity: layer.opacity, nearest, px_per_src, level: 0.0 });
    }

    /// A photo's cropped, adjusted, mipmapped texture (whole crop, pixel for
    /// pixel), kept by its look; it must fit the GPU (`Crop` w, h).
    fn image_tex(&mut self, id: LayerId, im: &crate::doc::ImageLayer, c: &Crop) -> Arc<wgpu::Texture> {
        let key = Key { id, content: c.key, rect: [c.w as i32, c.h as i32, 0, 0], view: [1; 4] };
        if let Some(e) = self.rasters.get_mut(&key) {
            e.1 = self.frame;
            return e.0.clone();
        }
        let t = self.prepared(im, [c.px[0] as u32, c.px[1] as u32], c.w as u32, c.h as u32);
        self.rasters.insert(key, (t.clone(), self.frame));
        t
    }

    /// A thin photo or video frame in a 3D space, sampled straight from its
    /// texture through the plane's homography `h` (content -> target pixel):
    /// nothing flattened on the CPU, so a playing video costs one upload a
    /// frame. None when the photo doesn't fit the GPU (it is flattened then).
    fn placed_image(&mut self, layer: &Layer, h: &crate::persp::M3, clip: Option<Rect>, v: &View) -> Option<Option<Node>> {
        let crate::doc::LayerKind::Image(im) = &layer.kind else { return None };
        let c = crop_of(im);
        if c.w as u32 > self.max_side || c.h as u32 > self.max_side {
            return None;
        }
        let tex = self.image_tex(layer.id, im, &c);
        // source pixel -> content (canvas) point, as `image` places it
        let (sn, cs) = im.rotation.sin_cos();
        let f = 1.0 / c.k;
        let place = Transform::from_row(cs * im.scale, sn * im.scale, -sn * im.scale, cs * im.scale, im.center.x, im.center.y).pre_translate(-c.w * f / 2.0, -c.h * f / 2.0).pre_scale(f, f);
        let a = [[place.sx as f64, place.kx as f64, place.tx as f64], [place.ky as f64, place.sy as f64, place.ty as f64], [0.0, 0.0, 1.0]];
        let Some(hi) = crate::persp::invert(&crate::persp::mul_mm(h, &a)) else { return Some(None) };
        let Some(q) = crate::persp::project_poly(h, &im.corners()) else { return Some(None) };
        let mut bb = Rect::from_points(&q);
        if let Some(cl) = clip {
            bb = bb.intersect(cl);
        }
        let r = [
            (bb.min.x.floor() as i32 - 1).max(0),
            (bb.min.y.floor() as i32 - 1).max(0),
            (bb.max.x.ceil() as i32 + 1).min(v.tw as i32),
            (bb.max.y.ceil() as i32 + 1).min(v.th as i32),
        ];
        if !bb.is_positive() || r[2] <= r[0] || r[3] <= r[1] {
            return Some(None);
        }
        Some(Some(Node::WarpImage { tex, hi: hi.map(|row| row.map(|x| x as f32)), rect: r, opacity: layer.opacity }))
    }

    /// A photo or video frame as it came (straight alpha; a decoded video frame
    /// turned into RGB on the GPU), kept by source id.
    fn source(&mut self, src: &crate::doc::ImageSource) -> Arc<wgpu::Texture> {
        if let Some(e) = self.sources.get_mut(&src.id) {
            e.1 = self.frame;
            return e.0.clone();
        }
        let t = match &src.nv12 {
            Some(f) => self.upload_nv12(&f.data, f.w, f.h),
            None => {
                let (w, h) = src.rgba.dimensions();
                self.upload(src.rgba.as_raw(), w, h)
            }
        };
        self.sources.insert(src.id, (t.clone(), self.frame));
        t
    }

    /// A photo's `w`×`h` pixels from `crop`, adjusted and premultiplied
    /// (`fs_prep`), with every mip level below it (`fs_mip`).
    fn prepared(&mut self, im: &crate::doc::ImageLayer, crop: [u32; 2], w: u32, h: u32) -> Arc<wgpu::Texture> {
        let raw = self.source(&im.src);
        let levels = 32 - w.max(h).max(1).leading_zeros();
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FMT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let level = |l: u32| tex.create_view(&wgpu::TextureViewDescriptor { base_mip_level: l, mip_level_count: Some(1), ..Default::default() });
        let mut enc = self.device.create_command_encoder(&Default::default());
        let a = &im.adjust;
        let mut u = Uniform::new([0.0, 0.0, w as f32, h as f32], w, h);
        u.p0 = [crop[0] as f32, crop[1] as f32, 0.0, 0.0];
        let tone = a.brightness != 0.0 || a.contrast != 0.0 || a.saturation != 0.0;
        u.p1 = [a.brightness, a.contrast, a.saturation, tone as u8 as f32];
        if let Some([r, g, b]) = a.chroma {
            u.h0 = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0];
            u.h1 = [a.key_tol, a.key_soft, 0.0, 0.0];
        }
        let dv = self.dummy.create_view(&Default::default());
        self.pass_views(&mut enc, &self.prep, &level(0), &u, &raw.create_view(&Default::default()), &dv);
        for l in 1..levels {
            let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
            let u = Uniform::new([0.0, 0.0, lw as f32, lh as f32], lw, lh);
            self.pass_views(&mut enc, &self.mip, &level(l), &u, &level(l - 1), &dv);
        }
        self.queue.submit([enc.finish()]);
        Arc::new(tex)
    }

    /// A brush layer on the canvas from its pages of tiles: each page in view
    /// sampled as a photo is (nearest at 1:1, bicubic otherwise), with hard
    /// edges, so neighbouring pages meet without a seam (and never overlap,
    /// so each takes the layer's opacity).
    fn atlas(&mut self, layer: &Layer, v: &View, out: &mut Vec<Node>) {
        use crate::raster::TILE;
        let side = (PAGE * TILE) as f32;
        let s = v.s;
        let nearest = (s - 1.0).abs() < 1e-4;
        for (tex, o) in self.pages_of(layer) {
            let (fx, fy) = ((o.x - v.full.min.x) * s, (o.y - v.full.min.y) * s);
            let inv = [[1.0 / s, 0.0, -fx / s], [0.0, 1.0 / s, -fy / s]];
            let rect = [(fx.floor() as i32).max(0), (fy.floor() as i32).max(0), ((fx + side * s).ceil() as i32).min(v.tw as i32), ((fy + side * s).ceil() as i32).min(v.th as i32)];
            if rect[2] <= rect[0] || rect[3] <= rect[1] {
                continue;
            }
            // px_per_src this large: the page's edge is a hard cut, not anti-aliased
            out.push(Node::Image { img: Img::Ready(tex), inv, rect, opacity: layer.opacity, nearest, px_per_src: 1e6, level: 0.0 });
        }
    }

    /// A brush layer's tiles in pages (made where tiles are, dropped where
    /// none are left, updated tile by tile): each page's texture and the
    /// canvas point of its top left.
    fn pages_of(&mut self, layer: &Layer) -> Vec<(Arc<wgpu::Texture>, Pos2)> {
        use crate::raster::TILE;
        let crate::doc::LayerKind::Raster(r) = &layer.kind else { return Vec::new() };
        let frame = self.frame;
        let page_of = |k: (i32, i32)| (k.0.div_euclid(PAGE), k.1.div_euclid(PAGE));
        let mut want: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
        for &k in r.tiles.keys() {
            want.entry(page_of(k)).or_default().push(k);
        }
        let side = (PAGE * TILE) as u32;
        let p = self.pages.entry(layer.id).or_insert_with(|| Paged { pages: HashMap::new(), used: frame });
        p.used = frame;
        p.pages.retain(|k, _| want.contains_key(k));
        let blank = vec![0u8; (TILE * TILE * 4) as usize];
        let mut out = Vec::new();
        for (pk, keys) in want {
            let page = p.pages.entry(pk).or_insert_with(|| {
                let t = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("brush page"),
                    size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FMT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                Page { tex: Arc::new(t), revs: HashMap::new() }
            });
            let put = |q: &wgpu::Queue, tex: &wgpu::Texture, key: (i32, i32), px: &[u8]| {
                q.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x: (key.0.rem_euclid(PAGE) * TILE) as u32, y: (key.1.rem_euclid(PAGE) * TILE) as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
                    px,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some((TILE * 4) as u32), rows_per_image: Some(TILE as u32) },
                    wgpu::Extent3d { width: TILE as u32, height: TILE as u32, depth_or_array_layers: 1 },
                );
            };
            // tiles gone: cleared; tiles new or changed: uploaded
            let gone: Vec<(i32, i32)> = page.revs.keys().copied().filter(|k| !r.tiles.contains_key(k)).collect();
            for k in gone {
                put(&self.queue, &page.tex, k, &blank);
                page.revs.remove(&k);
            }
            for k in keys {
                let t = &r.tiles[&k];
                if page.revs.get(&k) != Some(&t.rev) {
                    put(&self.queue, &page.tex, k, &t.px);
                    page.revs.insert(k, t.rev);
                }
            }
            // one canvas unit per page pixel
            out.push((page.tex.clone(), Pos2::new(r.offset.x + (pk.0 * PAGE * TILE) as f32, r.offset.y + (pk.1 * PAGE * TILE) as f32)));
        }
        out
    }

    /// A mask layer's tiles in one texture, updated tile by tile: the texture
    /// and the canvas point of its top left.
    fn atlas_of(&mut self, layer: &Layer) -> Option<(Arc<wgpu::Texture>, Pos2)> {
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
        // masks only now (brush layers use pages): at most 8192 px a side
        // (256 MB); a bigger mask is cut on the CPU instead. A 0.72.4 crash
        // was a whole-layer atlas near the 16384 limit running the GPU out of
        // memory while a long stroke made it again and again.
        let limit = (self.max_side.min(8192) / TILE as u32) as i32;
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
        // one canvas unit per atlas pixel
        let o = Pos2::new(r.offset.x + (a.t0.0 * TILE) as f32, r.offset.y + (a.t0.1 * TILE) as f32);
        Some((a.tex.clone(), o))
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
            Some((data, w, h))
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
        // thin photos and video frames: sampled straight from their texture
        if pl.thick <= 0.0 && std::env::var_os("DV_FLAT_IMAGES").is_none() {
            if let Some(n) = self.placed_image(&layer, &h, clip, v) {
                out.extend(n);
                return;
            }
        }
        // flat text and shapes: drawn flat on the GPU from their outlines (no
        // CPU picture, so turning and zooming make nothing on the CPU)
        let gpu_flat = match pl.thick <= 0.0 && crate::gpu_paths::handles(&layer) && std::env::var_os("DV_CPU_2D").is_none() {
            true => self.flat_paths(&layer, res),
            false => None,
        };
        let (tex, rect, fres, edges) = match gpu_flat {
            Some((t, r, f)) => (t, r, f, None),
            None => {
            // (not when it has got much too coarse: past half the scale needed, one
            // new picture is made even while zooming)
            let fl = match v.zooming {
                true => self.flat.peek(&layer, &pl, res).or_else(|| {
                    // the one it has, while the sharp one is made on a worker
                    let near = self.flat.nearest(&layer, &pl);
                    if near.is_some() {
                        self.flat.request(&layer, &pl, res);
                    }
                    near
                }),
                false => None,
            };
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
                (tex, fl.1, fl.2, fl.3.clone())
            }
        };
        let warp = |h: &crate::persp::M3, shade: f32, opacity: f32| -> Option<Node> {
            let hi = crate::persp::invert(h)?;
            let q = crate::persp::project_poly(h, &[rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()])?;
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
            Some(Node::Warp { img: Img::Ready(tex.clone()), hi, src: [rect.min.x, rect.min.y, fres], rect: r, opacity, shade })
        };
        if pl.thick <= 0.0 {
            out.extend(warp(&h, 1.0, layer.opacity));
            return;
        }
        let to_eye = |w: crate::persp::V3| crate::persp::sub(cam.center, w);
        if pl.engrave {
            // the floor, the inner walls over it, all seen through the shape
            let Some(Node::Warp { rect: or, .. }) = warp(&h, 1.0, 1.0) else { return };
            let (off, n) = crate::extrude::visible_cap(&pl, &to_eye);
            let hc = crate::persp::mul_mm(&out_m, &pl.homography_at(&cam, off));
            let mut inner: Vec<Node> = warp(&hc, light.map_or(1.0, |l| l.shade(n)), layer.opacity).into_iter().collect();
            if let Some(edges) = &edges {
                inner.extend(self.sides(edges, &layer, &pl, &cam, light.as_ref(), &out_m, rect, clip, v));
            }
            let opening = warp(&h, 1.0, 1.0).into_iter().collect();
            out.push(Node::Cut { inner, opening, rect: or });
            return;
        }
        if let Some(edges) = &edges {
            if let Some(n) = self.sides(edges, &layer, &pl, &cam, light.as_ref(), &out_m, rect, clip, v) {
                out.push(n);
            }
        }
        let (off, n) = crate::extrude::visible_cap(&pl, &to_eye);
        let hc = if off == 0.0 { h } else { crate::persp::mul_mm(&out_m, &pl.homography_at(&cam, off)) };
        out.extend(warp(&hc, light.map_or(1.0, |l| l.shade(n)), layer.opacity));
    }

    /// A flat (thin) text or shape layer in a 3D space drawn flat on the GPU at
    /// `res` texels per unit (steps of 2^(1/8), as `FlatCache`), over its bounds
    /// and a unit of margin, as `export::flatten_layer` makes it on the CPU.
    fn flat_paths(&mut self, layer: &Layer, res: f32) -> Option<(Arc<wgpu::Texture>, Rect, f32)> {
        let b = layer.bounds()?.expand(1.0);
        let res = 2f32.powf((res.max(1e-3).log2() * 8.0).ceil() / 8.0);
        let res = res.min(8192.0f32.min(self.max_side as f32) / b.width().max(b.height()).max(1.0));
        let w = (b.width() * res).ceil().max(1.0) as u32;
        let h = (b.height() * res).ceil().max(1.0) as u32;
        let key = Key { id: layer.id, content: crate::canvas::content_key(layer), rect: [w as i32, h as i32, 0, 0], view: [2, res.to_bits(), b.min.x.to_bits(), b.min.y.to_bits()] };
        if let Some(e) = self.rasters.get_mut(&key) {
            e.1 = self.frame;
            return Some((e.0.clone(), b, res));
        }
        let v = View { full: Rect::from_min_size(b.min, vec2(w as f32, h as f32) / res), s: res, tw: w, th: h, k: 1.0, origin: None, zooming: false };
        let t = self.raster_paths(layer, [0, 0, w as i32, h as i32], &v)?;
        self.rasters.insert(key, (t.clone(), self.frame));
        Some((t, b, res))
    }

    /// A thick layer's sides: its outline (uploaded once) and this frame's
    /// placement, camera and light as a uniform; the box is where the content
    /// rectangle's front and back corners land (the whole target when one is
    /// behind the camera).
    #[allow(clippy::too_many_arguments)]
    fn sides(&mut self, edges: &Arc<Vec<crate::extrude::Edge>>, layer: &Layer, pl: &crate::persp::Placement, cam: &crate::persp::Camera, light: Option<&crate::persp::Light>, out_m: &crate::persp::M3, rect: Rect, clip: Option<Rect>, v: &View) -> Option<Node> {
        use crate::persp::{mul_mm, mul_mv, sub};
        if edges.is_empty() {
            return None;
        }
        let ptr = Arc::as_ptr(edges) as usize;
        let frame = self.frame;
        let buf = match self.edge_bufs.get_mut(&ptr) {
            Some(e) => {
                e.2 = frame;
                e.1.clone()
            }
            None => {
                let mut bytes = Vec::with_capacity(edges.len() * EDGE_BYTES as usize);
                for e in edges.iter() {
                    for x in [e.a.x, e.a.y, e.b.x, e.b.y, e.n[0], e.n[1], e.na[0], e.na[1], e.nb[0], e.nb[1]] {
                        bytes.extend_from_slice(&x.to_le_bytes());
                    }
                    bytes.extend_from_slice(&[e.ca[0], e.ca[1], e.ca[2], 255, e.cb[0], e.cb[1], e.cb[2], 255]);
                }
                let b = Arc::new(self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &bytes, usage: wgpu::BufferUsages::VERTEX }));
                self.edge_bufs.insert(ptr, (edges.clone(), b.clone(), frame));
                b
            }
        };
        // (x, y, off, 1) -> homogeneous target pixel: out · K · R · [m | n | t − C − m·anchor]
        let (m, t) = pl.world_affine();
        let n = pl.normal();
        let k = [[cam.f, 0.0, cam.c[0]], [0.0, cam.f, cam.c[1]], [0.0, 0.0, 1.0]];
        let a = mul_mm(&mul_mm(out_m, &k), &cam.r);
        let (ax, ay) = (pl.anchor.x as f64, pl.anchor.y as f64);
        let c0 = sub(t, cam.center);
        let c0 = sub(c0, mul_mv(&m, [ax, ay, 0.0]));
        let cols = [mul_mv(&a, [m[0][0], m[1][0], m[2][0]]), mul_mv(&a, [m[0][1], m[1][1], m[2][1]]), mul_mv(&a, n), mul_mv(&a, c0)];
        let row = |i: usize| [cols[0][i] as f32, cols[1][i] as f32, cols[2][i] as f32, cols[3][i] as f32];
        let p = [row(0), row(1), row(2)];
        let at = |x: f32, y: f32, off: f32| {
            let q = [x, y, off, 1.0];
            let h = p.map(|r| r[0] * q[0] + r[1] * q[1] + r[2] * q[2] + r[3] * q[3]);
            (egui::pos2(h[0] / h[2], h[1] / h[2]), h[2])
        };
        let th = pl.thick as f32;
        let corners: Vec<(Pos2, f32)> = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].iter().flat_map(|c| [at(c.x, c.y, 0.0), at(c.x, c.y, th)]).collect();
        let near = corners.iter().map(|c| c.1).fold(f32::MAX, f32::min);
        let full = Rect::from_min_max(Pos2::ZERO, egui::pos2(v.tw as f32, v.th as f32));
        let bb = if near > 1e-6 { Rect::from_points(&corners.iter().map(|c| c.0).collect::<Vec<_>>()).expand(2.0).intersect(full) } else { full };
        let r = [(bb.min.x.floor() as i32).max(0), (bb.min.y.floor() as i32).max(0), (bb.max.x.ceil() as i32).min(v.tw as i32), (bb.max.y.ceil() as i32).min(v.th as i32)];
        let boxr = Rect::from_min_max(egui::pos2(r[0] as f32, r[1] as f32), egui::pos2(r[2] as f32, r[3] as f32));
        let sc = clip.map_or(boxr, |c| c.intersect(boxr));
        if r[2] <= r[0] || r[3] <= r[1] || !sc.is_positive() {
            return None;
        }
        let (ox, oy) = (r[0] as f32, r[1] as f32);
        let (bw, bh) = ((r[2] - r[0]) as u32, (r[3] - r[1]) as u32);
        let sx0 = ((sc.min.x.floor() - ox).max(0.0) as u32).min(bw - 1);
        let sy0 = ((sc.min.y.floor() - oy).max(0.0) as u32).min(bh - 1);
        let sx1 = ((sc.max.x.ceil() - ox).max(0.0) as u32).min(bw);
        let sy1 = ((sc.max.y.ceil() - oy).max(0.0) as u32).min(bh);
        let scissor = [sx0, sy0, sx1.saturating_sub(sx0).max(1), sy1.saturating_sub(sy0).max(1)];
        let row3 = |i: usize, w: f64| [m[i][0] as f32, m[i][1] as f32, m[i][2] as f32, w as f32];
        let dir = light.map_or([0.0; 3], |l| l.dir());
        let u = SideUniform {
            p0: p[0],
            p1: p[1],
            p2: p[2],
            m0: row3(0, t[0]),
            m1: row3(1, t[1]),
            m2: row3(2, t[2]),
            eye: [cam.center[0] as f32, cam.center[1] as f32, cam.center[2] as f32, th],
            nrm: [n[0] as f32, n[1] as f32, n[2] as f32, pl.smooth as u8 as f32],
            light: [dir[0] as f32, dir[1] as f32, dir[2] as f32, light.map_or(0.0, |l| l.ambient)],
            box_: [ox, oy, bw as f32, bh as f32],
            k: [(layer.opacity.clamp(0.0, 1.0) * 255.0).round() / 255.0, crate::canvas::SIDE_SHADE, light.is_some() as u8 as f32, if near > 1e-6 { near * 0.5 } else { 1e-4 }],
            anchor: [pl.anchor.x, pl.anchor.y, pl.engrave as u8 as f32, 0.0],
        };
        Some(Node::Sides { edges: buf, count: edges.len() as u32, u, rect: r, scissor })
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
        let (v0, v1) = (t0.create_view(&Default::default()), t1.create_view(&Default::default()));
        self.pass_views(enc, pipe, &target.create_view(&Default::default()), u, &v0, &v1);
    }

    /// As `pass`, on texture views (single mip levels).
    fn pass_views(&self, enc: &mut wgpu::CommandEncoder, pipe: &wgpu::RenderPipeline, view: &wgpu::TextureView, u: &Uniform, v0: &wgpu::TextureView, v1: &wgpu::TextureView) {
        let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: u.bytes(), usage: wgpu::BufferUsages::UNIFORM });
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(v0) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(v1) },
            ],
        });
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
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
            Node::Image { img, inv, rect, opacity, nearest, px_per_src, level } => {
                let t = Self::img(img, ready);
                let mut u = Uniform::new(rect.map(|x| x as f32), tw, th);
                u.tsize[2] = *opacity;
                u.h0 = [inv[0][0], inv[0][1], inv[0][2], 0.0];
                u.h1 = [inv[1][0], inv[1][1], inv[1][2], 0.0];
                u.p1 = [*px_per_src, *nearest as u8 as f32, *level, 0.0];
                self.pass(enc, &self.image, target, &u, &t, &self.dummy);
            }
            Node::WarpImage { tex, hi, rect, opacity } => {
                let mut u = Uniform::new(rect.map(|x| x as f32), tw, th);
                u.tsize[2] = *opacity;
                u.tsize[3] = 1.0;
                u.h0 = [hi[0][0], hi[0][1], hi[0][2], 0.0];
                u.h1 = [hi[1][0], hi[1][1], hi[1][2], 0.0];
                u.h2 = [hi[2][0], hi[2][1], hi[2][2], 0.0];
                u.p1[3] = (tex.mip_level_count() - 1) as f32;
                self.pass(enc, &self.warp_image, target, &u, tex, &self.dummy);
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
            Node::Sides { edges, count, u: su, rect, scissor } => {
                let (w, h) = ((rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32);
                // targets kept by size rounded up (a turning layer's box changes
                // every frame); the box is drawn in their top left
                let (pw, ph) = (w.div_ceil(128) * 128, h.div_ceil(128) * 128);
                let frame = self.frame;
                let (ms, depth, res, used) = self.ms_pool.entry((pw, ph)).or_insert_with(|| {
                    let ms_tex = |format: wgpu::TextureFormat| {
                        self.device.create_texture(&wgpu::TextureDescriptor {
                            label: None,
                            size: wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
                            mip_level_count: 1,
                            sample_count: 4,
                            dimension: wgpu::TextureDimension::D2,
                            format,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                            view_formats: &[],
                        })
                    };
                    let res = self.device.create_texture(&wgpu::TextureDescriptor {
                        label: None,
                        size: wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: FMT,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    });
                    (ms_tex(FMT), ms_tex(wgpu::TextureFormat::Depth32Float), res, frame)
                });
                *used = frame;
                let (ms, depth, res) = (ms.clone(), depth.clone(), res.clone());
                let ub = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: su.bytes(), usage: wgpu::BufferUsages::UNIFORM });
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.sides_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() }] });
                {
                    let (msv, rv, dv) = (ms.create_view(&Default::default()), res.create_view(&Default::default()), depth.create_view(&Default::default()));
                    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &msv,
                            depth_slice: None,
                            resolve_target: Some(&rv),
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &dv,
                            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    rp.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
                    rp.set_pipeline(&self.sides_pipe);
                    rp.set_bind_group(0, &bg, &[]);
                    rp.set_vertex_buffer(0, edges.slice(..));
                    rp.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
                    rp.draw(0..6, 0..*count);
                }
                let r = [rect[0] as f32, rect[1] as f32, rect[2] as f32, rect[3] as f32];
                let mut u = Uniform::new(r, tw, th);
                u.p0 = [rect[0] as f32, rect[1] as f32, 0.0, 0.0];
                self.pass(enc, &self.over, target, &u, &res, &self.dummy);
                // a big one isn't kept
                if pw * ph > 2048 * 2048 {
                    self.ms_pool.remove(&(pw, ph));
                }
            }
            Node::MaskedGpu { inner, atlas, a, rect, invert, strength } => {
                let g = self.pooled(tw, th, 2 + depth * 2);
                self.clear_to(enc, &g, wgpu::Color::TRANSPARENT);
                for n in inner {
                    self.run(enc, n, &g, depth + 1, ready, v, bgp);
                }
                let mut u = Uniform::new(rect.map(|x| x as f32), tw, th);
                u.h0 = [a[0][0], a[0][1], a[0][2], 0.0];
                u.h1 = [a[1][0], a[1][1], a[1][2], 0.0];
                u.p1 = [*invert as u8 as f32, *strength, 0.0, 0.0];
                self.pass(enc, &self.mask_atlas, target, &u, &g, atlas);
            }
            Node::Cut { inner, opening, rect } => {
                let g = self.pooled(tw, th, 2 + depth * 2);
                self.clear_to(enc, &g, wgpu::Color::TRANSPARENT);
                for n in inner {
                    self.run(enc, n, &g, depth + 1, ready, v, bgp);
                }
                // (a slot of its own: the inner nodes may use the next depth's)
                let o = self.pooled(tw, th, 1000 + depth);
                self.clear_to(enc, &o, wgpu::Color::TRANSPARENT);
                for n in opening {
                    self.run(enc, n, &o, depth + 1, ready, v, bgp);
                }
                let u = Uniform::new(rect.map(|x| x as f32), tw, th);
                self.pass(enc, &self.cut, target, &u, &g, &o);
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
/// Tiles per side of a brush layer's page (1024 px).
const PAGE: i32 = 4;

struct Page {
    tex: Arc<wgpu::Texture>,
    revs: HashMap<(i32, i32), u64>,
}

struct Paged {
    pages: HashMap<(i32, i32), Page>,
    used: u64,
}

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

fn u32_bytes(v: &[u32]) -> &[u8] {
    // plain u32s
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

/// Bytes per outline edge instance (`sides.wgsl` `In`): five f32 pairs, two
/// RGBA8 colours.
const EDGE_BYTES: u64 = 48;

/// `sides.wgsl`'s uniform.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SideUniform {
    p0: [f32; 4],
    p1: [f32; 4],
    p2: [f32; 4],
    m0: [f32; 4],
    m1: [f32; 4],
    m2: [f32; 4],
    eye: [f32; 4],
    nrm: [f32; 4],
    light: [f32; 4],
    box_: [f32; 4],
    k: [f32; 4],
    anchor: [f32; 4],
}

impl SideUniform {
    fn bytes(&self) -> &[u8] {
        // plain f32 arrays, no padding
        unsafe { std::slice::from_raw_parts(self as *const Self as *const u8, std::mem::size_of::<Self>()) }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    /// The canvas's picture of `doc` (`world_min`, `s`) as RGBA, w×h.
    fn canvas(g: &mut Gpu, doc: &Document, world_min: Pos2, s: f32, w: u32, h: u32) -> Vec<u8> {
        let out = g.texture(w, h, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING);
        g.render_canvas(doc, world_min, s, &out, None, false, None);
        let bpr = (w * 4).div_ceil(256) * 256;
        let buf = g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (bpr * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = g.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &out, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let idx = g.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = g.device.poll(wgpu::PollType::Wait { submission_index: Some(idx), timeout: None });
        let m = buf.slice(..).get_mapped_range().unwrap();
        m.chunks(bpr as usize).take(h as usize).flat_map(|r| r[..(w * 4) as usize].to_vec()).collect()
    }

    /// A brush stroke far longer than one texture (Ash, 10-22: past a certain
    /// width or height a stroke vanished): drawn whole on the canvas, from
    /// pages of tiles, without a seam between them.
    #[test]
    fn a_brush_layer_wider_than_a_texture_draws_whole() {
        let mut r = crate::raster::RasterLayer::default();
        // 100 tiles wide (25600 px), 40 px tall, from x = -300
        r.fill_rect(egui::Rect::from_min_max(pos2(-300.0, 0.0), pos2(25300.0, 40.0)), true);
        let mut doc = Document::default();
        doc.insert("b".into(), crate::doc::LayerKind::Raster(r), None);
        let ok = with_gpu(|g| {
            // the whole stroke in 512 px: 50 canvas units a pixel
            let (w, h) = (520u32, 8u32);
            let px = canvas(g, &doc, pos2(-400.0, -180.0), 1.0 / 50.0, w, h);
            let alpha = |x: u32, y: u32| px[((y * w + x) * 4 + 3) as usize];
            // row 3 crosses the stroke (y 0..40 → 3.6..4.4 px): every pixel
            // from its start (x 2) to its end (x 514) is covered, none after
            let row = 4;
            let covered = (3..513).filter(|&x| alpha(x, row) > 0).count();
            assert!(covered == 510, "covered {covered} of 510 along the stroke");
            assert_eq!(alpha(517, row), 0);
            // pages: 25600 px of tiles in pages of 1024
            assert!(g.pages.values().next().unwrap().pages.len() >= 25);
        });
        if ok.is_none() {
            eprintln!("no GPU: skipped");
        }
    }
}
