//! Glass cards under wgpu (render overhaul phase 3). egui-wgpu draws a frame in
//! one render pass, so nothing can copy the screen mid-frame as `glass.rs` does
//! under GL. Instead the cards sample the canvas texture (the document and its
//! ground, drawn by `gpu.rs` before egui paints): it is blurred once per frame
//! at quarter size (`update`), and each card is an egui-wgpu paint callback
//! with the GL version's look (`glass.wgsl`). Cards over other cards blur only
//! the canvas, and egui's overlay under a card is hidden: a shortcut of the
//! move to wgpu (10-20, not Ash's choice); Ash wants lower layers back
//! (10-27, see RENDER.md).

use std::sync::Mutex;

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu;
use egui::Rect;
use wgpu::util::DeviceExt;

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Shadow reach in points (as glass.rs).
const SHADOW: f32 = 22.0;

/// Pipelines and this frame's textures, kept in egui-wgpu's callback resources.
struct Res {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    copy: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    card: wgpu::RenderPipeline,
    /// half, quarter, quarter (ping-pong) of the canvas
    half: Option<wgpu::Texture>,
    quarter: Option<wgpu::Texture>,
    tmp: Option<wgpu::Texture>,
    /// this frame's canvas texture and where it sits (framebuffer px)
    scene: Option<(wgpu::TextureView, [f32; 4])>,
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
        half: None,
        quarter: None,
        tmp: None,
        scene: None,
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// This frame's canvas (the GPU canvas texture, its top left at `origin` in
/// framebuffer pixels): blurred for the cards.
pub fn update(rs: &egui_wgpu::RenderState, canvas: &wgpu::Texture, origin: [f32; 2]) {
    let device = &rs.device;
    let mut r = rs.renderer.write();
    let Some(res) = r.callback_resources.get_mut::<Res>() else { return };
    let (w, h) = (canvas.width(), canvas.height());
    let (hw, hh, qw, qh) = ((w / 2).max(1), (h / 2).max(1), (w / 4).max(1), (h / 4).max(1));
    let fits = |t: &Option<wgpu::Texture>, w: u32, h: u32| t.as_ref().is_some_and(|t| t.width() == w && t.height() == h);
    if !fits(&res.half, hw, hh) {
        res.half = Some(texture(device, hw, hh));
    }
    if !fits(&res.quarter, qw, qh) {
        res.quarter = Some(texture(device, qw, qh));
        res.tmp = Some(texture(device, qw, qh));
    }
    let (half, quarter, tmp) = (res.half.clone().unwrap(), res.quarter.clone().unwrap(), res.tmp.clone().unwrap());
    let mut enc = device.create_command_encoder(&Default::default());
    let pass = |enc: &mut wgpu::CommandEncoder, pipe: &wgpu::RenderPipeline, src: &wgpu::Texture, dst: &wgpu::Texture, dir: [f32; 2]| {
        let u = U { dir: [dir[0], dir[1], 0.0, 0.0], ..Default::default() };
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: u.bytes(), usage: wgpu::BufferUsages::UNIFORM });
        let sv = src.create_view(&Default::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &res.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&sv) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&sv) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&res.sampler) },
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
    };
    // 4×4 box average as two exact 2×2 steps, then a gaussian twice each way
    pass(&mut enc, &res.copy, canvas, &half, [0.0, 0.0]);
    pass(&mut enc, &res.copy, &half, &quarter, [0.0, 0.0]);
    for _ in 0..2 {
        pass(&mut enc, &res.blur, &quarter, &tmp, [1.4 / qw as f32, 0.0]);
        pass(&mut enc, &res.blur, &tmp, &quarter, [0.0, 1.4 / qh as f32]);
    }
    rs.queue.submit([enc.finish()]);
    res.scene = Some((canvas.create_view(&Default::default()), [origin[0], origin[1], w as f32, h as f32]));
}

/// A glass card (egui-wgpu callback) over `card` (points).
pub fn shape(card: Rect, look: crate::glass::Look) -> egui::Shape {
    egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(card.expand(SHADOW + 8.0), Card { card, look, bind: Mutex::new(None) }))
}

struct Card {
    card: Rect,
    look: crate::glass::Look,
    bind: Mutex<Option<wgpu::BindGroup>>,
}

impl CallbackTrait for Card {
    fn prepare(&self, device: &wgpu::Device, _queue: &wgpu::Queue, sd: &ScreenDescriptor, _enc: &mut wgpu::CommandEncoder, resources: &mut CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = resources.get::<Res>() else { return Vec::new() };
        let (Some((scene, map)), Some(quarter)) = (&res.scene, &res.quarter) else { return Vec::new() };
        let ppp = sd.pixels_per_point;
        let c = self.card.center();
        let k = crate::glass::strength();
        let u = U {
            panel: [c.x * ppp, c.y * ppp, self.card.width() * 0.5 * ppp, self.card.height() * 0.5 * ppp],
            tint: self.look.tint,
            p: [self.look.radius * ppp, crate::glass::band() * ppp, self.look.refract * ppp * k, self.look.dispersion * ppp * k],
            q: [SHADOW * ppp, if crate::glass::shadow() { 1.0 } else { 0.0 }, ppp, if res.srgb { 1.0 } else { 0.0 }],
            map: *map,
            dir: [0.0; 4],
        };
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: u.bytes(), usage: wgpu::BufferUsages::UNIFORM });
        let qv = quarter.create_view(&Default::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &res.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(scene) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&qv) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&res.sampler) },
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
