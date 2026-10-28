//! The frame description: what a document shows at its current time, as a flat
//! list of draw items in drawing order. Built once per picture and read by the
//! renderers (the CPU export now; the GPU renderer next), so every decision
//! about how a layer is drawn (projected, grouped, masked, blended) is made in
//! one place. See docs/RENDER.md.

use crate::doc::{Blend, Document, FxLayer, Layer, LayerKind};
use crate::export::MaskMap;
use crate::persp::{Camera, Light, Placement};
use egui::Rect;

pub struct Scene {
    pub items: Vec<Item>,
}

/// One top-level layer: where it shows (canvas space, for culling) and how it
/// is drawn.
#[derive(Clone)]
pub struct Item {
    pub bounds: Rect,
    pub draw: Draw,
}

#[derive(Clone)]
pub enum Draw {
    /// A 2D layer drawn as it is, at its opacity (put down as one picture when
    /// translucent, so overlapping parts don't add up).
    Flat(Layer),
    /// Text with a blend mode: drawn alone, then blended onto what is below at
    /// the layer's opacity.
    Blend(Layer, Blend),
    /// Drawn alone at full strength, then put down once with a blend mode and
    /// opacity (thick layers, placed text and brush layers with a blend mode).
    Group { inner: Box<Draw>, mode: Blend, opacity: f32 },
    /// Drawn alone, then cut by a mask layer (`map`: where an anchored mask has
    /// moved to).
    Masked { inner: Box<Draw>, mask: Layer, map: Option<MaskMap> },
    /// A layer in a 3D space: flattened, then projected through its camera.
    Projected(Box<Projected>),
    /// An FX layer: everything drawn so far, filtered and put back (복사 or a
    /// blend mode), mixed with the original by `opacity`, only where its mask
    /// lets it (docs/updates/fx-layers.md).
    /// `frame`: what vignette and colour fringes centre on (the export
    /// frame, else the content's bounds).
    Fx { fx: FxLayer, opacity: f32, mask: Option<(Layer, Option<MaskMap>)>, frame: Rect },
}

#[derive(Clone)]
pub struct Projected {
    pub layer: Layer,
    pub pl: Placement,
    pub cam: Camera,
    /// The space's clip rectangle (canvas space).
    pub clip: Option<Rect>,
    /// The space's light, for thick layers.
    pub light: Option<Light>,
}

/// A camera of the 3D workspace standing in for a space's own: only that
/// space's layers are drawn, through it, unclipped (free views).
pub struct ViewCam {
    pub space: u64,
    pub cam: Camera,
}

/// The document at its current time (`anim::evaluate` has posed it).
pub fn build(doc: &Document) -> Scene {
    build_view(doc, None)
}

/// As `build`, or through a view camera.
pub fn build_view(doc: &Document, view: Option<&ViewCam>) -> Scene {
    let cams = match view {
        Some(v) => std::iter::once((v.space, v.cam.clone())).collect(),
        None => crate::persp::cameras(&doc.spaces),
    };
    let mut items = Vec::new();
    let frame = doc.export_frame.or_else(|| doc.content_bounds()).unwrap_or(Rect::from_center_size(egui::Pos2::ZERO, egui::vec2(1000.0, 1000.0)));
    for layer in doc.draw_order(&cams).into_iter().filter(|l| l.visible && l.opacity > 0.0) {
        if layer.mask_of.is_some() {
            continue;
        }
        if view.is_some_and(|v| layer.place.as_ref().is_none_or(|pl| pl.space != v.space)) {
            continue;
        }
        let timed = crate::anim::layer_at_time(doc, layer).unwrap_or_else(|| layer.clone());
        // a filter of the whole picture so far (never in a space)
        if let LayerKind::Fx(fx) = &timed.kind {
            if fx.is_noop() && fx.blend.is_none() {
                continue;
            }
            let mask = crate::export::mask_for(doc, timed.id).map(|m| (m.clone(), crate::export::mask_map(doc, m)));
            items.push(Item { bounds: Rect::EVERYTHING, draw: Draw::Fx { fx: fx.clone(), opacity: timed.opacity, mask, frame } });
            continue;
        }
        let Some(bounds) = timed.shown_bounds(&cams) else { continue };
        let mask = crate::export::mask_for(doc, timed.id).map(|m| (m.clone(), crate::export::mask_map(doc, m)));
        let draw = match mask {
            Some((mask, map)) => masked(classify(doc, &cams, timed, view.is_some()), mask, map),
            None => classify(doc, &cams, timed, view.is_some()),
        };
        items.push(Item { bounds, draw });
    }
    Scene { items }
}

/// A mask on a layer. The layer is cut by the mask first and only then mixed
/// with what is below: a blend mode (or a group's opacity) goes outside the
/// mask, else it would mix with nothing but the layer's own empty buffer.
fn masked(d: Draw, mask: Layer, map: Option<MaskMap>) -> Draw {
    match d {
        Draw::Group { inner, mode, opacity } => Draw::Group { inner: Box::new(Draw::Masked { inner, mask, map }), mode, opacity },
        Draw::Blend(mut layer, mode) => {
            let opacity = layer.opacity;
            layer.opacity = 1.0;
            if let LayerKind::Text(t) = &mut layer.kind {
                t.blend = Blend::Normal;
            }
            Draw::Group { inner: Box::new(Draw::Masked { inner: Box::new(Draw::Flat(layer)), mask, map }), mode, opacity }
        }
        d => Draw::Masked { inner: Box::new(d), mask, map },
    }
}

fn classify(doc: &Document, cams: &std::collections::HashMap<u64, Camera>, layer: Layer, free: bool) -> Draw {
    if let Some((mode, opacity)) = crate::canvas::group_of(&layer) {
        let mut one = layer;
        one.opacity = 1.0;
        match &mut one.kind {
            LayerKind::Text(t) => t.blend = Blend::Normal,
            LayerKind::Raster(r) => r.blend = Blend::Normal,
            _ => {}
        }
        return Draw::Group { inner: Box::new(classify(doc, cams, one, free)), mode, opacity };
    }
    if let Some((pl, cam)) = layer.place.as_ref().and_then(|pl| cams.get(&pl.space).map(|c| (pl.clone(), c.clone()))) {
        let space = doc.spaces.iter().find(|s| s.id == pl.space);
        let clip = space.filter(|_| !free).map(|s| s.clip());
        let light = space.and_then(|s| s.light).filter(|_| pl.thick > 0.0);
        return Draw::Projected(Box::new(Projected { layer, pl, cam, clip, light }));
    }
    if let LayerKind::Text(t) = &layer.kind {
        if t.blend != Blend::Normal {
            let b = t.blend;
            return Draw::Blend(layer, b);
        }
    }
    Draw::Flat(layer)
}
