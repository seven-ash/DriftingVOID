//! Glass cards under wgpu (render overhaul phase 3). egui-wgpu draws a frame in
//! one render pass, so nothing can copy the screen mid-frame as `glass.rs` did
//! under GL. Instead, before egui paints:
//! - the canvas texture (the document and its ground, drawn by `gpu.rs`) is
//!   blurred once per frame at quarter size (`update`);
//! - egui's overlay on the canvas (the rail's tool fill, selection outlines)
//!   is drawn into a half-size texture, a light blur laid over the glass after
//!   its tint, so it is not darkened or split (Ash, 10-15);
//! - a card over anything drawn before it gets a grab: everything up to its
//!   layer (canvas, overlay, lower cards with their own glass) drawn into a
//!   texture and blurred like the canvas, so what lies under it shows through
//!   (Ash, 10-27; only the blur stacks, 10-15). See `backdrop`.
//!
//! Each card is an egui-wgpu paint callback with the GL version's look
//! (`glass.wgsl`).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu;
use egui::Rect;
use wgpu::util::DeviceExt;

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Shadow reach in points (as glass.rs).
const SHADOW: f32 = 22.0;
/// At most this many grabs a frame; cards past them use the last one before.
const MAX_GRABS: usize = 3;

/// A blurred picture for the cards: its full-size source and the blur chain.
struct Blurred {
    half: wgpu::Texture,
    quarter: wgpu::Texture,
    tmp: wgpu::Texture,
}

/// A grab: everything drawn before a card's layer (framebuffer size, in the
/// window's format, multisampled like egui) and its blur.
struct Grab {
    msaa: wgpu::Texture,
    screen: wgpu::Texture,
    /// `screen` read without sRGB decoding (the canvas is sampled raw too)
    view: wgpu::TextureView,
    blur: Blurred,
}

/// Pipelines and this frame's textures, kept in egui-wgpu's callback resources.
struct Res {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    copy: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    card: wgpu::RenderPipeline,
    /// the canvas's blur chain
    canvas: Option<Blurred>,
    /// this frame's canvas texture and where it sits (framebuffer px)
    scene: Option<(wgpu::TextureView, [f32; 4])>,
    /// the overlay: drawn multisampled at full size, kept at half size
    overlay: Option<(wgpu::Texture, wgpu::Texture, wgpu::Texture)>,
    /// whether the overlay holds anything this frame
    overlay_on: bool,
    grabs: Vec<Grab>,
    /// how many grabs this frame
    grab_n: usize,
    /// a clear 1×1 texture for unused overlay slots
    clear: wgpu::Texture,
    /// framebuffer size this frame
    fb: [f32; 2],
    /// drawing a grab: cards in it give only their blur, so tint, refraction
    /// and dispersion apply once, by the card on top (Ash, 10-15)
    in_grab: bool,
    /// the canvas's egui texture (left out of the overlay)
    canvas_id: Option<egui::TextureId>,
    target: wgpu::TextureFormat,
    msaa: u32,
    srgb: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct U {
    panel: [f32; 4],
    tint: [f32; 4],
    p: [f32; 4],
    q: [f32; 4],
    map: [f32; 4],
    dir: [f32; 4],
}

impl U {
    fn bytes(&self) -> &[u8] {
        // plain f32 arrays, no padding
        unsafe { std::slice::from_raw_parts(self as *const Self as *const u8, std::mem::size_of::<Self>()) }
    }
}

/// Sets the pipelines up (eframe on wgpu); `msaa` is egui's sample count.
pub fn install(rs: &egui_wgpu::RenderState, msaa: u32) {
    let device = &rs.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("glass"), source: wgpu::ShaderSource::Wgsl(include_str!("glass.wgsl").into()) });
    let tex = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("glass"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            tex(1),
            tex(2),
            wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            tex(4),
        ],
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
    let pipe = |entry: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>, samples: u32| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            multiview_mask: None,
            cache: None,
        })
    };
    let over = wgpu::BlendState {
        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
    };
    let clear = texture(device, 1, 1);
    rs.queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &clear, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &[0u8; 4],
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );
    let res = Res {
        copy: pipe("fs_copy", FMT, None, 1),
        blur: pipe("fs_blur", FMT, None, 1),
        card: pipe("fs_card", rs.target_format, Some(over), msaa.max(1)),
        sampler: device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        }),
        layout,
        canvas: None,
        scene: None,
        overlay: None,
        overlay_on: false,
        grabs: Vec::new(),
        grab_n: 0,
        clear,
        fb: [1.0, 1.0],
        in_grab: false,
        canvas_id: None,
        target: rs.target_format,
        msaa: msaa.max(1),
        srgb: rs.target_format.is_srgb(),
    };
    rs.renderer.write().callback_resources.insert(res);
    crate::glass::set_ready(true);
}

fn texture(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glass"),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        // COPY_DST: the clear 1×1 texture is written once at start
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// A texture egui can draw into: the window's format with `samples`, and
/// readable without sRGB decoding.
fn target_texture(device: &wgpu::Device, format: wgpu::TextureFormat, w: u32, h: u32, samples: u32) -> wgpu::Texture {
    let raw = format.remove_srgb_suffix();
    let views = [raw];
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glass grab"),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: if samples > 1 { wgpu::TextureUsages::RENDER_ATTACHMENT } else { wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING },
        view_formats: if raw != format { &views } else { &[] },
    })
}

fn raw_view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor { format: Some(t.format().remove_srgb_suffix()), ..Default::default() })
}

fn fits(t: &wgpu::Texture, w: u32, h: u32) -> bool {
    t.width() == w.max(1) && t.height() == h.max(1)
}

/// The blur chain for a `w`×`h` picture (kept while the size holds).
fn blurred(device: &wgpu::Device, old: Option<Blurred>, w: u32, h: u32) -> Blurred {
    let (hw, hh, qw, qh) = ((w / 2).max(1), (h / 2).max(1), (w / 4).max(1), (h / 4).max(1));
    match old {
        Some(b) if fits(&b.half, hw, hh) && fits(&b.quarter, qw, qh) => b,
        _ => Blurred { half: texture(device, hw, hh), quarter: texture(device, qw, qh), tmp: texture(device, qw, qh) },
    }
}

/// One full-screen pass of `pipe` from `src` into `dst`.
fn pass(device: &wgpu::Device, res: &Res, enc: &mut wgpu::CommandEncoder, pipe: &wgpu::RenderPipeline, src: &wgpu::TextureView, dst: &wgpu::Texture, dir: [f32; 2]) {
    let u = U { dir: [dir[0], dir[1], 0.0, 0.0], ..Default::default() };
    let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: u.bytes(), usage: wgpu::BufferUsages::UNIFORM });
    let cv = res.clear.create_view(&Default::default());
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &res.layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(src) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&res.sampler) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&cv) },
        ],
    });
    let dv = dst.create_view(&Default::default());
    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &dv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
        ..Default::default()
    });
    rp.set_pipeline(pipe);
    rp.set_bind_group(0, &bg, &[]);
    rp.draw(0..3, 0..1);
}

/// 4×4 box average as two exact 2×2 steps, then a gaussian twice each way.
fn blur_into(device: &wgpu::Device, res: &Res, enc: &mut wgpu::CommandEncoder, src: &wgpu::TextureView, b: &Blurred) {
    let (qw, qh) = (b.quarter.width(), b.quarter.height());
    pass(device, res, enc, &res.copy, src, &b.half, [0.0, 0.0]);
    pass(device, res, enc, &res.copy, &b.half.create_view(&Default::default()), &b.quarter, [0.0, 0.0]);
    for _ in 0..2 {
        pass(device, res, enc, &res.blur, &b.quarter.create_view(&Default::default()), &b.tmp, [1.4 / qw as f32, 0.0]);
        pass(device, res, enc, &res.blur, &b.tmp.create_view(&Default::default()), &b.quarter, [0.0, 1.4 / qh as f32]);
    }
}

/// This frame's canvas (the GPU canvas texture `id`, its top left at `origin`
/// in framebuffer pixels): blurred for the cards.
pub fn update(rs: &egui_wgpu::RenderState, canvas: &wgpu::Texture, id: egui::TextureId, origin: [f32; 2]) {
    let device = &rs.device;
    let mut r = rs.renderer.write();
    let Some(res) = r.callback_resources.get_mut::<Res>() else { return };
    let (w, h) = (canvas.width(), canvas.height());
    res.canvas = Some(blurred(device, res.canvas.take(), w, h));
    res.canvas_id = Some(id);
    let mut enc = device.create_command_encoder(&Default::default());
    blur_into(device, res, &mut enc, &canvas.create_view(&Default::default()), res.canvas.as_ref().unwrap());
    rs.queue.submit([enc.finish()]);
    res.scene = Some((canvas.create_view(&Default::default()), [origin[0], origin[1], w as f32, h as f32]));
}

/// This frame's glass cards: their callback (to find their layer), rect and
/// the backdrop level `backdrop` gives them.
static CARDS: Mutex<Vec<(usize, Rect, Arc<AtomicU32>)>> = Mutex::new(Vec::new());

/// A glass card (egui-wgpu callback) over `card` (points).
pub fn shape(card: Rect, look: crate::glass::Look) -> egui::Shape {
    let level = Arc::new(AtomicU32::new(0));
    let cb = egui_wgpu::Callback::new_paint_callback(card.expand(SHADOW + 8.0), Card { card, look, level: level.clone(), bind: Mutex::new(None) });
    if let Ok(mut c) = CARDS.lock() {
        c.push((Arc::as_ptr(&cb.callback) as *const () as usize, card, level));
    }
    egui::Shape::Callback(cb)
}

fn card_key(s: &egui::Shape) -> Option<usize> {
    match s {
        egui::Shape::Callback(pc) => Some(Arc::as_ptr(&pc.callback) as *const () as usize),
        _ => None,
    }
}

/// Builds what the cards show through them, once a frame after the UI is laid
/// out and before egui paints (see the module notes). In paint order, a card
/// whose rect meets anything an earlier layer drew gets a grab of all the
/// layers before its own; the rest show the canvas and the overlay.
pub fn backdrop(ctx: &egui::Context, rs: &egui_wgpu::RenderState) {
    let t0 = std::time::Instant::now();
    backdrop_inner(ctx, rs);
    // DV_GLASS_DEBUG=1: its CPU time, averaged over 60 passes
    if std::env::var_os("DV_GLASS_DEBUG").is_some() {
        static ACC: Mutex<(f64, u32)> = Mutex::new((0.0, 0));
        if let Ok(mut a) = ACC.lock() {
            a.0 += t0.elapsed().as_secs_f64() * 1e3;
            a.1 += 1;
            if a.1 == 60 {
                eprintln!("glass backdrop: {:.3} ms a pass (CPU, 60 passes)", a.0 / 60.0);
                *a = (0.0, 0);
            }
        }
    }
}

fn backdrop_inner(ctx: &egui::Context, rs: &egui_wgpu::RenderState) {
    let cards = CARDS.lock().map(|mut c| std::mem::take(&mut *c)).unwrap_or_default();
    if cards.is_empty() || !crate::glass::available() {
        return;
    }
    let canvas_id = {
        let r = rs.renderer.read();
        match r.callback_resources.get::<Res>() {
            Some(res) if res.scene.is_some() => res.canvas_id,
            _ => return,
        }
    };
    let ppp = ctx.pixels_per_point();
    let screen = ctx.content_rect();
    let (fw, fh) = ((screen.width() * ppp).round() as u32, (screen.height() * ppp).round() as u32);

    // the layers in paint order with their shapes: the canvas's own layers
    // first, then the areas (as egui's `GraphicLayers::drain`)
    let order: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
    let bg = [egui::LayerId::background(), egui::LayerId::new(egui::Order::Background, egui::Id::new("text_edit_bg"))];
    let mut seq: Vec<egui::LayerId> = Vec::new();
    for o in [egui::Order::Background, egui::Order::Middle, egui::Order::Foreground, egui::Order::Tooltip, egui::Order::Debug] {
        if o == egui::Order::Background {
            seq.extend(bg.iter().copied().filter(|l| !order.contains(l)));
        }
        seq.extend(order.iter().copied().filter(|l| l.order == o));
    }
    let layers: Vec<(egui::LayerId, Vec<egui::epaint::ClippedShape>)> =
        ctx.graphics(|g| seq.iter().filter_map(|&l| g.get(l).map(|pl| (l, pl.all_entries().cloned().collect::<Vec<_>>()))).filter(|(_, s)| !s.is_empty()).collect());

    // levels: 0 = canvas and overlay; k = the k-th grab
    let mut cutoffs: Vec<usize> = Vec::new();
    let mut drawn: Vec<Rect> = Vec::new();
    for (li, (l, shapes)) in layers.iter().enumerate() {
        let on_canvas = l.order == egui::Order::Background;
        let mut bounds = Rect::NOTHING;
        for cs in shapes {
            match card_key(&cs.shape).and_then(|k| cards.iter().find(|c| c.0 == k)) {
                Some((_, rect, level)) => {
                    if !on_canvas && drawn.iter().any(|d| d.intersects(*rect)) {
                        if cutoffs.last() != Some(&li) && cutoffs.len() < MAX_GRABS {
                            cutoffs.push(li);
                        }
                        level.store(cutoffs.iter().filter(|&&c| c <= li).count() as u32, Ordering::Relaxed);
                    } else {
                        level.store(0, Ordering::Relaxed);
                    }
                    bounds = bounds.union(*rect);
                }
                None => bounds = bounds.union(cs.shape.visual_bounding_rect().intersect(cs.clip_rect)),
            }
        }
        if !on_canvas && bounds.is_positive() {
            drawn.push(bounds);
        }
    }

    // DV_GLASS_DEBUG=1: the layers with cards and the grabs, on stderr
    if std::env::var_os("DV_GLASS_DEBUG").is_some() && ctx.cumulative_pass_nr() % 60 == 30 {
        let lv: Vec<u32> = cards.iter().map(|c| c.2.load(Ordering::Relaxed)).collect();
        eprintln!("glass: {} layers, cards {} levels {:?}, grabs at {:?}", layers.len(), cards.len(), lv, cutoffs.iter().map(|&c| layers[c].0).collect::<Vec<_>>());
    }

    // what to draw: the overlay (canvas layers without the canvas picture),
    // then each grab (every layer before its cutoff)
    let overlay: Vec<egui::epaint::ClippedShape> = layers
        .iter()
        .filter(|(l, _)| l.order == egui::Order::Background)
        .flat_map(|(_, s)| s.iter().cloned())
        .filter(|cs| !matches!(&cs.shape, egui::Shape::Mesh(m) if Some(m.texture_id) == canvas_id))
        .collect();
    let sd = ScreenDescriptor { size_in_pixels: [fw, fh], pixels_per_point: ppp };
    let device = &rs.device;
    let queue = &rs.queue;
    let mut r = rs.renderer.write();
    {
        let Some(res) = r.callback_resources.get_mut::<Res>() else { return };
        res.fb = [fw as f32, fh as f32];
        let (target, msaa) = (res.target, res.msaa);
        let need_overlay = match &res.overlay {
            Some((m, _, _)) => !fits(m, fw, fh),
            None => true,
        };
        if need_overlay {
            res.overlay = Some((target_texture(device, target, fw, fh, msaa), target_texture(device, target, fw, fh, 1), texture(device, (fw / 2).max(1), (fh / 2).max(1))));
        }
        while res.grabs.len() < cutoffs.len() {
            let b = blurred(device, None, fw, fh);
            let screen = target_texture(device, target, fw, fh, 1);
            res.grabs.push(Grab { msaa: target_texture(device, target, fw, fh, msaa), view: raw_view(&screen), screen, blur: b });
        }
        for g in res.grabs.iter_mut().take(cutoffs.len()) {
            if !fits(&g.screen, fw, fh) {
                g.msaa = target_texture(device, target, fw, fh, msaa);
                g.screen = target_texture(device, target, fw, fh, 1);
                g.view = raw_view(&g.screen);
            }
            let b = std::mem::replace(&mut g.blur, blurred(device, None, 1, 1));
            g.blur = blurred(device, Some(b), fw, fh);
        }
        res.grab_n = cutoffs.len();
        res.overlay_on = !overlay.is_empty();
    }
    // egui's renderer has one vertex buffer: each drawing is submitted on its own
    let draw = |r: &mut egui_wgpu::Renderer, shapes: Vec<egui::epaint::ClippedShape>, msaa: &wgpu::Texture, out: &wgpu::Texture, after: &dyn Fn(&Res, &mut wgpu::CommandEncoder)| {
        let jobs = ctx.tessellate(shapes, ppp);
        let mut enc = device.create_command_encoder(&Default::default());
        let cbs = r.update_buffers(device, queue, &mut enc, &jobs, &sd);
        {
            let mv = msaa.create_view(&Default::default());
            let ov = out.create_view(&Default::default());
            let (view, resolve) = if msaa.sample_count() > 1 { (&mv, Some(&ov)) } else { (&ov, None) };
            let mut rp = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("glass backdrop"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: resolve, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                    ..Default::default()
                })
                .forget_lifetime();
            r.render(&mut rp, &jobs, &sd);
        }
        if let Some(res) = r.callback_resources.get::<Res>() {
            after(res, &mut enc);
        }
        queue.submit(cbs.into_iter().chain([enc.finish()]));
    };
    let (om, ofull) = {
        let res = r.callback_resources.get::<Res>().unwrap();
        let (m, f, _) = res.overlay.as_ref().unwrap();
        (m.clone(), f.clone())
    };
    if !overlay.is_empty() {
        // the overlay at half size: the light blur
        draw(&mut r, overlay, &om, &ofull, &|res, enc| {
            let (_, full, half) = res.overlay.as_ref().unwrap();
            pass(device, res, enc, &res.copy, &raw_view(full), half, [0.0, 0.0]);
        });
    }
    if let Some(res) = r.callback_resources.get_mut::<Res>() {
        res.in_grab = true;
    }
    for (k, &cut) in cutoffs.iter().enumerate() {
        let shapes: Vec<egui::epaint::ClippedShape> = layers[..cut].iter().flat_map(|(_, s)| s.iter().cloned()).collect();
        let (gm, gs) = {
            let g = &r.callback_resources.get::<Res>().unwrap().grabs[k];
            (g.msaa.clone(), g.screen.clone())
        };
        draw(&mut r, shapes, &gm, &gs, &|res, enc| {
            let g = &res.grabs[k];
            blur_into(device, res, enc, &g.view, &g.blur);
        });
    }
    if let Some(res) = r.callback_resources.get_mut::<Res>() {
        res.in_grab = false;
    }
}

struct Card {
    card: Rect,
    look: crate::glass::Look,
    /// 0: the canvas and the overlay; k: the k-th grab (set by `backdrop`)
    level: Arc<AtomicU32>,
    bind: Mutex<Option<wgpu::BindGroup>>,
}

impl CallbackTrait for Card {
    fn prepare(&self, device: &wgpu::Device, _queue: &wgpu::Queue, sd: &ScreenDescriptor, _enc: &mut wgpu::CommandEncoder, resources: &mut CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = resources.get::<Res>() else { return Vec::new() };
        let level = self.level.load(Ordering::Relaxed) as usize;
        let grab = (level > 0 && level <= res.grab_n).then(|| &res.grabs[level - 1]);
        let (sharp, quarter, map) = match (grab, &res.scene, &res.canvas) {
            (Some(g), _, _) => (g.view.clone(), &g.blur.quarter, [0.0, 0.0, res.fb[0], res.fb[1]]),
            (None, Some((scene, map)), Some(b)) => (scene.clone(), &b.quarter, *map),
            _ => return Vec::new(),
        };
        // the overlay goes over canvas-level cards; a grab already holds it
        let overlay = match (&res.overlay, grab.is_none() && res.overlay_on) {
            (Some((_, _, half)), true) => half.create_view(&Default::default()),
            _ => res.clear.create_view(&Default::default()),
        };
        let ppp = sd.pixels_per_point;
        let c = self.card.center();
        // inside a grab only the blur (the card on top tints and refracts once)
        let k = if res.in_grab { 0.0 } else { crate::glass::strength() };
        let mut tint = self.look.tint;
        if res.in_grab {
            tint[3] = 0.0;
        }
        let u = U {
            panel: [c.x * ppp, c.y * ppp, self.card.width() * 0.5 * ppp, self.card.height() * 0.5 * ppp],
            tint,
            p: [self.look.radius * ppp, crate::glass::band() * ppp, self.look.refract * ppp * k, self.look.dispersion * ppp * k],
            q: [SHADOW * ppp, if crate::glass::shadow() && !res.in_grab { 1.0 } else { 0.0 }, ppp, if res.srgb { 1.0 } else { 0.0 }],
            map,
            dir: [sd.size_in_pixels[0] as f32, sd.size_in_pixels[1] as f32, 0.0, 0.0],
        };
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: u.bytes(), usage: wgpu::BufferUsages::UNIFORM });
        let qv = quarter.create_view(&Default::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &res.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&sharp) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&qv) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&res.sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&overlay) },
            ],
        });
        if let Ok(mut b) = self.bind.lock() {
            *b = Some(bg);
        }
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, rp: &mut wgpu::RenderPass<'static>, resources: &CallbackResources) {
        let Some(res) = resources.get::<Res>() else { return };
        let Some(bg) = self.bind.lock().ok().and_then(|b| b.clone()) else { return };
        rp.set_pipeline(&res.card);
        rp.set_bind_group(0, &bg, &[]);
        rp.draw(0..3, 0..1);
    }
}
