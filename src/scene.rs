//! The frame description: what a document shows at its current time, as a flat
//! list of draw items in drawing order. Built once per picture and read by the
//! renderers (the CPU export now; the GPU renderer next), so every decision
//! about how a layer is drawn (projected, grouped, masked, blended) is made in
//! one place. See docs/updates/wgpu-renderer.md (phase 1).

use crate::doc::{Blend, Document, Layer, LayerKind};
use crate::export::MaskMap;
use crate::persp::{Camera, Light, Placement};
use egui::Rect;

pub struct Scene {
    pub items: Vec<Item>,
}

/// One top-level layer: where it shows (canvas space, for culling) and how it
/// is drawn.
pub struct Item {
    pub bounds: Rect,
    pub draw: Draw,
}

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
}

pub struct Projected {
    pub layer: Layer,
    pub pl: Placement,
    pub cam: Camera,
    /// The space's clip rectangle (canvas space).
    pub clip: Option<Rect>,
    /// The space's light, for thick layers.
    pub light: Option<Light>,
}

/// The document at its current time (`anim::evaluate` has posed it).
pub fn build(doc: &Document) -> Scene {
    let cams = crate::persp::cameras(&doc.spaces);
    let mut items = Vec::new();
    for layer in doc.draw_order(&cams).into_iter().filter(|l| l.visible && l.opacity > 0.0) {
        if layer.mask_of.is_some() {
            continue;
        }
        let timed = crate::anim::layer_at_time(doc, layer).unwrap_or_else(|| layer.clone());
        let Some(bounds) = timed.shown_bounds(&cams) else { continue };
        let mask = crate::export::mask_for(doc, timed.id).map(|m| (m.clone(), crate::export::mask_map(doc, m)));
        let draw = match mask {
            Some((mask, map)) => Draw::Masked { inner: Box::new(classify(doc, &cams, timed)), mask, map },
            None => classify(doc, &cams, timed),
        };
        items.push(Item { bounds, draw });
    }
    Scene { items }
}

fn classify(doc: &Document, cams: &std::collections::HashMap<u64, Camera>, layer: Layer) -> Draw {
    if let Some((mode, opacity)) = crate::canvas::group_of(&layer) {
        let mut one = layer;
        one.opacity = 1.0;
        match &mut one.kind {
            LayerKind::Text(t) => t.blend = Blend::Normal,
            LayerKind::Raster(r) => r.blend = Blend::Normal,
            _ => {}
        }
        return Draw::Group { inner: Box::new(classify(doc, cams, one)), mode, opacity };
    }
    if let Some((pl, cam)) = layer.place.as_ref().and_then(|pl| cams.get(&pl.space).map(|c| (pl.clone(), c.clone()))) {
        let space = doc.spaces.iter().find(|s| s.id == pl.space);
        let clip = space.map(|s| s.clip());
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
