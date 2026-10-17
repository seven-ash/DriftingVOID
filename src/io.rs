//! Image import and the `.void` project format (a zip with doc.json, the
//! original photo files and raw raster tiles).

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;

use egui::{Color32, Rect, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::doc::*;
use crate::raster::{RasterLayer, TILE, Tile};
use crate::vector::VStroke;

pub fn decode_image(bytes: Vec<u8>, ext_hint: Option<&str>) -> Result<ImageSource, String> {
    use image::ImageDecoder;
    let reader = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let fmt = reader.format();
    let mut dec = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = dec.orientation().ok();
    let mut img = image::DynamicImage::from_decoder(dec).map_err(|e| e.to_string())?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    let ext = fmt
        .and_then(|f| f.extensions_str().first().map(|s| s.to_string()))
        .or_else(|| ext_hint.map(|s| s.to_string()))
        .unwrap_or_else(|| "bin".into());
    Ok(ImageSource { id: next_rev(), rgba: Arc::new(img.to_rgba8()), encoded: Arc::new(bytes), ext })
}

pub fn load_image_file(path: &Path) -> Result<ImageSource, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    decode_image(bytes, path.extension().and_then(|e| e.to_str())).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn image_from_rgba(img: image::RgbaImage) -> ImageSource {
    let mut png = Vec::new();
    let _ = image::DynamicImage::ImageRgba8(img.clone()).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png);
    ImageSource { id: next_rev(), rgba: Arc::new(img), encoded: Arc::new(png), ext: "png".into() }
}

pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff"];

// ---------------------------------------------------------------- project

#[derive(Serialize, Deserialize)]
struct FileDoc {
    version: u32,
    next_id: u64,
    export_frame: Option<[f32; 4]>,
    layers: Vec<FileLayer>,
    #[serde(default)]
    spaces: Vec<crate::persp::Space>,
}

#[derive(Serialize, Deserialize)]
struct FileLayer {
    id: u64,
    name: String,
    visible: bool,
    locked: bool,
    opacity: f32,
    kind: FileKind,
}

#[derive(Serialize, Deserialize)]
enum FileKind {
    Image { file: String, center: [f32; 2], scale: f32, rotation: f32, crop: [f32; 4], adjust: Adjust },
    Text {
        pos: [f32; 2],
        text: String,
        size: f32,
        color: [u8; 4],
        #[serde(default)]
        family: String,
        #[serde(default)]
        style: String,
        bg: Option<[u8; 4]>,
        wrap: f32,
        #[serde(default)]
        rotation: f32,
        #[serde(default)]
        blend: Blend,
    },
    Shape {
        kind: ShapeKind,
        a: [f32; 2],
        b: [f32; 2],
        color: [u8; 4],
        width: f32,
        fill: Option<[u8; 4]>,
        #[serde(default)]
        rotation: f32,
    },
    Vector { offset: [f32; 2], strokes: Vec<FileStroke> },
    Raster { offset: [f32; 2], tiles: Vec<[i32; 2]> },
}

#[derive(Serialize, Deserialize)]
struct FileStroke {
    color: [u8; 4],
    pts: Vec<[f32; 3]>,
}

fn c2a(c: Color32) -> [u8; 4] {
    c.to_srgba_unmultiplied()
}
fn a2c(a: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(a[0], a[1], a[2], a[3])
}
fn r2a(r: Rect) -> [f32; 4] {
    [r.min.x, r.min.y, r.max.x, r.max.y]
}
fn a2r(a: [f32; 4]) -> Rect {
    Rect::from_min_max(pos2(a[0], a[1]), pos2(a[2], a[3]))
}

pub fn save_project(doc: &Document, path: &Path) -> Result<(), String> {
    let tmp = path.with_extension("void.tmp");
    {
        let f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(f));
        let deflate = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let mut written: HashMap<u64, String> = HashMap::new();
        let mut layers = Vec::new();
        for l in &doc.layers {
            let kind = match &l.kind {
                LayerKind::Image(im) => {
                    let file = match written.get(&im.src.id) {
                        Some(f) => f.clone(),
                        None => {
                            let f = format!("images/{}.{}", im.src.id, im.src.ext);
                            zip.start_file(&f, stored).map_err(|e| e.to_string())?;
                            zip.write_all(&im.src.encoded).map_err(|e| e.to_string())?;
                            written.insert(im.src.id, f.clone());
                            f
                        }
                    };
                    FileKind::Image {
                        file,
                        center: [im.center.x, im.center.y],
                        scale: im.scale,
                        rotation: im.rotation,
                        crop: r2a(im.crop),
                        adjust: im.adjust,
                    }
                }
                LayerKind::Text(t) => FileKind::Text {
                    pos: [t.pos.x, t.pos.y],
                    text: t.text.clone(),
                    size: t.size,
                    color: c2a(t.color),
                    family: t.family.clone(),
                    style: t.style.clone(),
                    bg: t.bg.map(c2a),
                    wrap: t.wrap,
                    rotation: t.rotation,
                    blend: t.blend,
                },
                LayerKind::Shape(s) => FileKind::Shape {
                    kind: s.kind,
                    a: [s.a.x, s.a.y],
                    b: [s.b.x, s.b.y],
                    color: c2a(s.color),
                    width: s.width,
                    fill: s.fill.map(c2a),
                    rotation: s.rotation,
                },
                LayerKind::Vector(v) => FileKind::Vector {
                    offset: [v.offset.x, v.offset.y],
                    strokes: v.strokes.iter().map(|s| FileStroke { color: c2a(s.color), pts: s.pts.clone() }).collect(),
                },
                LayerKind::Raster(r) => {
                    let mut keys = Vec::new();
                    for (&(x, y), t) in &r.tiles {
                        if t.is_empty() {
                            continue;
                        }
                        zip.start_file(format!("tiles/{}/{}_{}.raw", l.id, x, y), deflate).map_err(|e| e.to_string())?;
                        zip.write_all(&t.px).map_err(|e| e.to_string())?;
                        keys.push([x, y]);
                    }
                    FileKind::Raster { offset: [r.offset.x, r.offset.y], tiles: keys }
                }
            };
            layers.push(FileLayer { id: l.id, name: l.name.clone(), visible: l.visible, locked: l.locked, opacity: l.opacity, kind });
        }
        let fd = FileDoc { version: 1, next_id: doc.next_id, export_frame: doc.export_frame.map(r2a), layers, spaces: doc.spaces.clone() };
        zip.start_file("doc.json", deflate).map_err(|e| e.to_string())?;
        zip.write_all(&serde_json::to_vec_pretty(&fd).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        zip.finish().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn load_project(path: &Path) -> Result<Document, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| e.to_string())?;
    let mut read = |name: &str| -> Result<Vec<u8>, String> {
        let mut e = zip.by_name(name).map_err(|e| format!("{name}: {e}"))?;
        let mut v = Vec::new();
        e.read_to_end(&mut v).map_err(|e| e.to_string())?;
        Ok(v)
    };
    let fd: FileDoc = serde_json::from_slice(&read("doc.json")?).map_err(|e| e.to_string())?;
    let mut images: HashMap<String, ImageSource> = HashMap::new();
    let mut doc = Document { layers: Vec::new(), export_frame: fd.export_frame.map(a2r), next_id: fd.next_id, spaces: fd.spaces };
    for fl in fd.layers {
        let kind = match fl.kind {
            FileKind::Image { file, center, scale, rotation, crop, adjust } => {
                let src = match images.get(&file) {
                    Some(s) => s.clone(),
                    None => {
                        let s = decode_image(read(&file)?, None)?;
                        images.insert(file, s.clone());
                        s
                    }
                };
                LayerKind::Image(ImageLayer { src, center: pos2(center[0], center[1]), scale, rotation, crop: a2r(crop), adjust })
            }
            FileKind::Text { pos, text, size, color, family, style, bg, wrap, rotation, blend } => LayerKind::Text(TextLayer {
                pos: pos2(pos[0], pos[1]),
                text,
                size,
                color: a2c(color),
                family,
                style,
                bg: bg.map(a2c),
                wrap,
                rotation,
                blend,
            }),
            FileKind::Shape { kind, a, b, color, width, fill, rotation } => LayerKind::Shape(ShapeLayer {
                kind,
                a: pos2(a[0], a[1]),
                b: pos2(b[0], b[1]),
                color: a2c(color),
                width,
                fill: fill.map(a2c),
                rotation,
            }),
            FileKind::Vector { offset, strokes } => LayerKind::Vector(VectorLayer {
                offset: vec2(offset[0], offset[1]),
                strokes: strokes.into_iter().map(|s| Arc::new(VStroke::new(s.pts, a2c(s.color)))).collect(),
            }),
            FileKind::Raster { offset, tiles } => {
                let mut r = RasterLayer { offset: vec2(offset[0], offset[1]), tiles: Default::default() };
                for [x, y] in tiles {
                    let px = read(&format!("tiles/{}/{}_{}.raw", fl.id, x, y))?;
                    if px.len() == (TILE * TILE * 4) as usize {
                        r.tiles.insert((x, y), Arc::new(Tile { rev: next_rev(), px }));
                    }
                }
                LayerKind::Raster(r)
            }
        };
        doc.next_id = doc.next_id.max(fl.id);
        doc.layers.push(Layer { id: fl.id, name: fl.name, visible: fl.visible, locked: fl.locked, opacity: fl.opacity, kind });
    }
    Ok(doc)
}
