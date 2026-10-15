//! Headless check of the document -> project file -> export pipeline.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Color32, Rect, pos2, vec2};

use crate::doc::*;
use crate::raster::{BrushParams, RasterLayer, RasterStroke};
use crate::vector::{VStroke, radius_for};

pub fn run(out: &Path, photo: Option<PathBuf>) -> Result<(), String> {
    let res = build_and_export(out, photo);
    let _ = std::fs::write(out.with_extension("log"), match &res {
        Ok(s) => format!("ok\n{s}"),
        Err(e) => format!("error: {e}"),
    });
    res.map(|_| ())
}

fn build_and_export(out: &Path, photo: Option<PathBuf>) -> Result<String, String> {
    let mut doc = Document::default();
    let src = match photo {
        Some(p) => crate::io::load_image_file(&p)?,
        None => {
            let img = image::RgbaImage::from_fn(640, 420, |x, y| {
                image::Rgba([(x * 255 / 640) as u8, (y * 255 / 420) as u8, 160, 255])
            });
            crate::io::image_from_rgba(img)
        }
    };
    let mut im = ImageLayer::new(src, pos2(400.0, 300.0));
    im.scale = 600.0 / im.src.size().x;
    im.rotation = -4f32.to_radians();
    im.crop = Rect::from_min_max(pos2(0.0, 0.0), im.src.size().to_pos2() - vec2(0.0, im.src.size().y * 0.15));
    im.adjust.saturation = 0.2;
    doc.insert("photo".into(), LayerKind::Image(im), None);

    doc.insert(
        "text".into(),
        LayerKind::Text(TextLayer {
            pos: pos2(120.0, 560.0),
            text: "여기 너무 좋았어!\nDrifting VOID 123".into(),
            size: 36.0,
            color: Color32::from_rgb(30, 30, 30),
            family: std::env::var("SB_FONT").unwrap_or_default(),
            style: std::env::var("SB_STYLE").unwrap_or_else(|_| "Bold".into()),
            bg: Some(Color32::from_rgba_unmultiplied(255, 236, 150, 230)),
            wrap: 0.0,
        }),
        None,
    );
    for (kind, a, b) in [
        (ShapeKind::Arrow, pos2(520.0, 620.0), pos2(640.0, 470.0)),
        (ShapeKind::Ellipse, pos2(560.0, 140.0), pos2(700.0, 240.0)),
        (ShapeKind::Rect, pos2(80.0, 80.0), pos2(220.0, 180.0)),
    ] {
        doc.insert(
            "shape".into(),
            LayerKind::Shape(ShapeLayer { kind, a, b, color: Color32::from_rgb(230, 57, 70), width: 5.0, fill: None }),
            None,
        );
    }

    // vector: a pressure wave (handwriting-like)
    let mut pts = Vec::new();
    for i in 0..200 {
        let t = i as f32 / 199.0;
        let x = 80.0 + t * 640.0;
        let y = 700.0 + (t * 18.0).sin() * 25.0;
        let pressure = (t * std::f32::consts::PI).sin();
        pts.push([x, y, radius_for(4.0, pressure, true)]);
    }
    let mut v = VectorLayer::default();
    v.strokes.push(Arc::new(VStroke::new(pts, Color32::from_rgb(58, 134, 255))));
    doc.insert("pen".into(), LayerKind::Vector(v), None);

    // raster: soft brush loop + an eraser cut through it
    let mut r = RasterLayer::default();
    let brush = |r: &mut RasterLayer, erase: bool, color: Color32, radius: f32, path: &[(f32, f32, f32)]| {
        let mut st = RasterStroke::new(BrushParams {
            color,
            radius,
            opacity: 0.8,
            hardness: 0.3,
            pressure_size: true,
            pressure_opacity: false,
            erase,
        });
        for &(x, y, p) in path {
            st.add_point(r, pos2(x, y), p);
        }
        st.finish(r);
    };
    let circle: Vec<(f32, f32, f32)> = (0..=120)
        .map(|i| {
            let a = i as f32 / 120.0 * std::f32::consts::TAU;
            (640.0 + a.cos() * 90.0, 560.0 + a.sin() * 60.0, 0.4 + 0.6 * (a * 2.0).sin().abs())
        })
        .collect();
    brush(&mut r, false, Color32::from_rgb(82, 183, 136), 14.0, &circle);
    brush(&mut r, true, Color32::BLACK, 12.0, &[(540.0, 560.0, 1.0), (740.0, 560.0, 1.0)]);
    doc.insert("brush".into(), LayerKind::Raster(r), None);

    // group transform check: copies of every layer, scaled 0.5 and rotated 30 degrees
    // about a shared centre, shifted to the right of the originals
    if std::env::var_os("SB_GROUP").is_some() {
        let c = doc.content_bounds().unwrap().center();
        let copies: Vec<Layer> = doc
            .layers
            .iter()
            .map(|l| {
                let mut t = crate::app::transform_layer(l, c, 0.5, 30f32.to_radians());
                t.translate(vec2(700.0, 0.0));
                t
            })
            .collect();
        for mut l in copies {
            l.id = doc.new_id();
            doc.layers.push(l);
        }
    }

    // sub-selection check: lift the left half of the brush ring and move it up,
    // and move every other pen stroke point-set (here: the only stroke) right
    if std::env::var_os("SB_SUB").is_some() {
        use crate::app::{Part, apply_part};
        let ids: Vec<(LayerId, bool)> = doc
            .layers
            .iter()
            .filter_map(|l| match l.kind {
                LayerKind::Raster(_) => Some((l.id, true)),
                LayerKind::Vector(_) => Some((l.id, false)),
                _ => None,
            })
            .collect();
        for (id, raster) in ids {
            let i = doc.index_of(id).unwrap();
            let o = doc.layers[i].clone();
            let part = if raster {
                Part::Pixels(vec![Rect::from_min_max(pos2(520.0, 480.0), pos2(640.0, 640.0))])
            } else {
                Part::Strokes(vec![0])
            };
            let d = if raster { vec2(0.0, -120.0) } else { vec2(60.0, 0.0) };
            doc.layers[i] = apply_part(&o, &part, egui::Pos2::ZERO, 1.0, 0.0, d);
        }
    }

    // round-trip through the project format
    let proj = out.with_extension("void");
    crate::io::save_project(&doc, &proj)?;
    let doc2 = crate::io::load_project(&proj)?;
    if doc2.layers.len() != doc.layers.len() {
        return Err("layer count mismatch after reload".into());
    }

    let region = doc2.content_bounds().ok_or("no content")?.expand(20.0);
    let pm = crate::export::render(&doc2, region, 1.0, Some(Color32::from_rgb(250, 247, 240)))?;
    crate::export::save(&pm, out, 92)?;
    let pick = crate::export::sample(&doc2, pos2(400.0, 300.0));
    Ok(format!("export {}x{} layers {} sample {:?}", pm.width(), pm.height(), doc2.layers.len(), pick))
}
