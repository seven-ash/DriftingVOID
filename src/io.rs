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
    // HEIC/HEIF (phone photos): through ffmpeg, which the app has anyway; the
    // original file is kept as it is, as for every photo
    if is_heif(&bytes) {
        let img = crate::video::decode_heif(&bytes)?;
        return Ok(ImageSource { id: next_rev(), rgba: Arc::new(img), encoded: Arc::new(bytes), ext: "heic".into(), logical: None, nv12: None });
    }
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
    Ok(ImageSource { id: next_rev(), rgba: Arc::new(img.to_rgba8()), encoded: Arc::new(bytes), ext, logical: None, nv12: None })
}

pub fn load_image_file(path: &Path) -> Result<ImageSource, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    decode_image(bytes, path.extension().and_then(|e| e.to_str())).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn image_from_rgba(img: image::RgbaImage) -> ImageSource {
    let mut png = Vec::new();
    let _ = image::DynamicImage::ImageRgba8(img.clone()).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png);
    ImageSource { id: next_rev(), rgba: Arc::new(img), encoded: Arc::new(png), ext: "png".into(), logical: None, nv12: None }
}

pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "heic", "heif"];

/// An HEIF still (HEIC from phones): the ISO box `ftyp` with an HEIF brand.
pub fn is_heif(b: &[u8]) -> bool {
    b.len() >= 12 && &b[4..8] == b"ftyp" && matches!(&b[8..12], b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1")
}

// ---------------------------------------------------------------- project

#[derive(Serialize, Deserialize)]
struct FileDoc {
    version: u32,
    next_id: u64,
    export_frame: Option<[f32; 4]>,
    layers: Vec<FileLayer>,
    #[serde(default)]
    spaces: Vec<crate::persp::Space>,
    #[serde(default)]
    timeline: crate::anim::Timeline,
    #[serde(default)]
    cameras: Vec<crate::anim::CanvasCam>,
    /// Sound clips of files before 0.54.0; they load as sound layers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    audio: Vec<crate::anim::AudioClip>,
}

#[derive(Serialize, Deserialize)]
struct FileLayer {
    id: u64,
    name: String,
    visible: bool,
    locked: bool,
    opacity: f32,
    kind: FileKind,
    #[serde(default)]
    place: Option<crate::persp::Placement>,
    #[serde(default)]
    anim: Option<crate::anim::Anim>,
    #[serde(default)]
    mask_of: Option<LayerId>,
    #[serde(default)]
    mask_anchor: Option<(LayerId, [f32; 2])>,
    #[serde(default)]
    mask_pose: Option<[f32; 2]>,
    #[serde(default)]
    mask_invert: bool,
    #[serde(default = "unit_res")]
    mask_res: f32,
}

fn unit_res() -> f32 {
    1.0
}

#[derive(Serialize, Deserialize)]
enum FileKind {
    Image {
        file: String,
        center: [f32; 2],
        scale: f32,
        rotation: f32,
        crop: [f32; 4],
        adjust: Adjust,
        /// Video layers: the file reference; `file` then holds the current frame (PNG).
        #[serde(default)]
        video: Option<VideoRef>,
    },
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
        #[serde(default)]
        align: crate::doc::TextAlign,
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
    Audio(crate::doc::AudioLayer),
    Fx(crate::doc::FxLayer),
    Raster {
        offset: [f32; 2],
        tiles: Vec<[i32; 2]>,
        #[serde(default)]
        blend: Blend,
        /// Recorded brush strokes and the pixels from before them (`tiles_base/`).
        #[serde(default)]
        recs: Vec<crate::raster::BrushRec>,
        #[serde(default)]
        base_tiles: Option<Vec<[i32; 2]>>,
    },
}

#[derive(Serialize, Deserialize)]
struct FileStroke {
    color: [u8; 4],
    pts: Vec<[f32; 3]>,
    #[serde(default)]
    rec: Option<crate::vector::Rec>,
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
                            // video frames have no file bytes: store the frame as PNG
                            let png;
                            let (bytes, ext): (&[u8], &str) = if im.src.encoded.is_empty() {
                                let mut v = Vec::new();
                                let _ = image::DynamicImage::ImageRgba8((*im.src.rgba).clone()).write_to(&mut Cursor::new(&mut v), image::ImageFormat::Png);
                                png = v;
                                (&png, "png")
                            } else {
                                (&im.src.encoded, &im.src.ext)
                            };
                            let f = format!("images/{}.{}", im.src.id, ext);
                            zip.start_file(&f, stored).map_err(|e| e.to_string())?;
                            zip.write_all(bytes).map_err(|e| e.to_string())?;
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
                        video: im.video.clone(),
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
                    align: t.align,
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
                    strokes: v.strokes.iter().map(|s| FileStroke { color: c2a(s.color), pts: s.pts.clone(), rec: s.rec.clone() }).collect(),
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
                    let base_tiles = match &r.rec_base {
                        Some(base) => {
                            let mut bk = Vec::new();
                            for (&(x, y), t) in base {
                                if t.is_empty() {
                                    continue;
                                }
                                zip.start_file(format!("tiles_base/{}/{}_{}.raw", l.id, x, y), deflate).map_err(|e| e.to_string())?;
                                zip.write_all(&t.px).map_err(|e| e.to_string())?;
                                bk.push([x, y]);
                            }
                            Some(bk)
                        }
                        None => None,
                    };
                    FileKind::Raster { offset: [r.offset.x, r.offset.y], tiles: keys, blend: r.blend, recs: r.recs.iter().map(|x| (**x).clone()).collect(), base_tiles }
                }
                LayerKind::Audio(a) => FileKind::Audio(a.clone()),
                LayerKind::Fx(fx) => FileKind::Fx(fx.clone()),
            };
            layers.push(FileLayer { id: l.id, name: l.name.clone(), visible: l.visible, locked: l.locked, opacity: l.opacity, kind, place: l.place.clone(), anim: l.anim.clone(), mask_of: l.mask_of, mask_anchor: l.mask_anchor, mask_pose: l.mask_pose, mask_invert: l.mask_invert, mask_res: l.mask_res });
        }
        let fd = FileDoc { version: 1, next_id: doc.next_id, export_frame: doc.export_frame.map(r2a), layers, spaces: doc.spaces.clone(), timeline: doc.timeline.clone(), cameras: doc.cameras.clone(), audio: Vec::new() };
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
    let mut doc = Document { layers: Vec::new(), export_frame: fd.export_frame.map(a2r), next_id: fd.next_id, spaces: fd.spaces, timeline: fd.timeline, cameras: fd.cameras };
    let old_audio = fd.audio;
    for fl in fd.layers {
        let kind = match fl.kind {
            FileKind::Image { file, center, scale, rotation, crop, adjust, video } => {
                let src = match images.get(&file) {
                    Some(s) => s.clone(),
                    None => {
                        let s = decode_image(read(&file)?, None)?;
                        images.insert(file, s.clone());
                        s
                    }
                };
                let mut src = src;
                if let Some(v) = &video {
                    src.logical = Some([v.info.width, v.info.height]);
                }
                LayerKind::Image(ImageLayer { src, center: pos2(center[0], center[1]), scale, rotation, crop: a2r(crop), adjust, video })
            }
            FileKind::Text { pos, text, size, color, family, style, bg, wrap, rotation, blend, align } => LayerKind::Text(TextLayer {
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
                align,
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
                strokes: strokes.into_iter().map(|s| Arc::new(VStroke::new(s.pts, a2c(s.color)).with_rec(s.rec))).collect(),
            }),
            FileKind::Raster { offset, tiles, blend, recs, base_tiles } => {
                let mut r = RasterLayer { offset: vec2(offset[0], offset[1]), recs: recs.into_iter().map(Arc::new).collect(), blend, ..Default::default() };
                if let Some(bk) = base_tiles {
                    let mut base = crate::doc::TileMap::default();
                    for [x, y] in bk {
                        let px = read(&format!("tiles_base/{}/{}_{}.raw", fl.id, x, y))?;
                        if px.len() == (TILE * TILE * 4) as usize {
                            base.insert((x, y), Arc::new(Tile { rev: next_rev(), px }));
                        }
                    }
                    r.rec_base = Some(base);
                }
                for [x, y] in tiles {
                    let px = read(&format!("tiles/{}/{}_{}.raw", fl.id, x, y))?;
                    if px.len() == (TILE * TILE * 4) as usize {
                        r.tiles.insert((x, y), Arc::new(Tile { rev: next_rev(), px }));
                    }
                }
                LayerKind::Raster(r)
            }
            FileKind::Audio(a) => LayerKind::Audio(a),
            FileKind::Fx(fx) => LayerKind::Fx(fx),
        };
        doc.next_id = doc.next_id.max(fl.id);
        doc.layers.push(Layer { id: fl.id, name: fl.name, visible: fl.visible, locked: fl.locked, opacity: fl.opacity, kind, place: fl.place, anim: fl.anim, mask_of: fl.mask_of, mask_anchor: fl.mask_anchor, mask_pose: fl.mask_pose, mask_invert: fl.mask_invert, mask_res: fl.mask_res });
    }
    clips_to_layers(&mut doc, old_audio);
    crate::persp::to_eye_level(&mut doc);
    Ok(doc)
}

/// Sound clips of older files become sound layers: same time, trim, volume and
/// keys; their gizmos stand in a column left of the content.
fn clips_to_layers(doc: &mut Document, clips: Vec<crate::anim::AudioClip>) {
    if clips.is_empty() {
        return;
    }
    let content = doc.layers.iter().filter_map(|l| l.bounds()).reduce(|a, b| a.union(b)).unwrap_or(Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::ZERO));
    let size = crate::doc::AudioLayer::SIZE;
    for (i, c) in clips.into_iter().enumerate() {
        let pos = pos2(content.min.x - size * 1.5, content.min.y + size * 0.5 + i as f32 * size * 1.5);
        let kind = LayerKind::Audio(crate::doc::AudioLayer { path: c.path, pos, len: 0.0, volume: c.gain, mute: c.mute });
        let mut anim = c.anim;
        anim.span = Some([c.start, c.start + c.len]);
        anim.video_start = c.offset;
        let id = doc.new_id();
        doc.layers.push(Layer { id, name: c.name, visible: true, locked: false, opacity: 1.0, kind, place: None, anim: Some(anim), mask_of: None, mask_anchor: None, mask_pose: None, mask_invert: false, mask_res: 1.0 });
    }
}

#[cfg(test)]
mod video_tests {
    use super::*;

    #[test]
    fn video_layer_round_trips() {
        let info = crate::video::VideoInfo { width: 1920, height: 1080, fps: 30.0, duration: 8.0, audio: true, codec: "h264".into(), alpha: false };
        // a preview-sized frame without file bytes, as video frames are
        let src = ImageSource { id: next_rev(), rgba: Arc::new(image::RgbaImage::new(96, 54)), encoded: Arc::new(Vec::new()), ext: "png".into(), logical: Some([1920, 1080]), nv12: None };
        let mut im = ImageLayer::new(src, pos2(10.0, 20.0));
        im.video = Some(VideoRef { path: "C:/v/clip.mp4".into(), info: info.clone(), time: 2.5, speed: 2.0, looping: true, mute: false, volume: 1.0, slot: 1 });
        assert_eq!(im.crop.size(), vec2(1920.0, 1080.0));
        let mut doc = Document::default();
        doc.insert("v".into(), LayerKind::Image(im), None);
        let path = std::env::temp_dir().join("dv_video_roundtrip.void");
        save_project(&doc, &path).unwrap();
        let back = load_project(&path).unwrap();
        let LayerKind::Image(b) = &back.layers[0].kind else { panic!() };
        let v = b.video.as_ref().unwrap();
        assert_eq!((v.path.as_str(), v.time, v.speed, v.looping), ("C:/v/clip.mp4", 2.5, 2.0, true));
        assert_eq!(v.info, info);
        assert_eq!(b.src.size(), vec2(1920.0, 1080.0));
        assert_eq!(b.src.rgba.dimensions(), (96, 54));
    }

    #[test]
    fn recordings_round_trip() {
        use crate::raster::{BrushParams, BrushRec, RasterLayer, RasterStroke};
        let params = BrushParams { color: Color32::RED, radius: 4.0, opacity: 1.0, hardness: 1.0, pressure_size: false, pressure_opacity: false, erase: false };
        let mut r = RasterLayer { rec_base: Some(Default::default()), ..Default::default() };
        let pts = vec![[10.0, 10.0, 1.0], [40.0, 10.0, 1.0]];
        let mut st = RasterStroke::new(params);
        for p in &pts {
            st.add_point(&mut r, pos2(p[0], p[1]), p[2]);
        }
        st.finish(&mut r);
        r.recs.push(Arc::new(BrushRec { start: 1.5, params, pts, times: vec![0.0, 0.4] }));
        let mut doc = Document::default();
        doc.insert("b".into(), LayerKind::Raster(r), None);
        let v = VStroke::timed(vec![[0.0, 0.0, 1.0], [5.0, 0.0, 1.0]], vec![0.0, 0.3], Color32::WHITE, 2.0);
        doc.insert("p".into(), LayerKind::Vector(VectorLayer { offset: vec2(0.0, 0.0), strokes: vec![Arc::new(v)] }), None);
        let path = std::env::temp_dir().join("dv_rec_roundtrip.void");
        save_project(&doc, &path).unwrap();
        let back = load_project(&path).unwrap();
        let LayerKind::Raster(br) = &back.layers[0].kind else { panic!() };
        assert_eq!((br.recs.len(), br.recs[0].start, br.recs[0].times.clone()), (1, 1.5, vec![0.0, 0.4]));
        assert!(br.rec_base.is_some());
        let LayerKind::Vector(bv) = &back.layers[1].kind else { panic!() };
        assert_eq!(bv.strokes[0].rec.as_ref().unwrap().times, vec![0.0, 0.3]);
    }

    /// An FX layer keeps its filter, values, blend and keys through a save,
    /// and its keyed value plays back.
    #[test]
    fn fx_layer_roundtrip() {
        use crate::doc::{Filter, FxLayer};
        let mut doc = Document::default();
        let id = doc.insert("fx".into(), LayerKind::Fx(FxLayer { filter: Filter::BrightContrast, v: [0.2, -0.3, 0.0, 0.0], blend: Some(Blend::Multiply) }), None);
        let mut an = crate::anim::Anim::default();
        for (t, b) in [(0.0, 0.0f32), (2.0, 0.8)] {
            let mut k = crate::anim::key_of(doc.get(id).unwrap(), t);
            k.ch = crate::anim::ch::FX[0];
            k.fx[0] = b;
            an.keys.push(k);
        }
        doc.get_mut(id).unwrap().anim = Some(an);
        let path = std::env::temp_dir().join("dv_fx_roundtrip.void");
        save_project(&doc, &path).unwrap();
        let mut back = load_project(&path).unwrap();
        let LayerKind::Fx(fx) = &back.layers[0].kind else { panic!() };
        assert_eq!((fx.filter, fx.v, fx.blend), (Filter::BrightContrast, [0.2, -0.3, 0.0, 0.0], Some(Blend::Multiply)));
        back.timeline.on = true;
        back.timeline.time = 1.0;
        crate::anim::evaluate(&mut back);
        let LayerKind::Fx(fx) = &back.layers[0].kind else { panic!() };
        assert!((fx.v[0] - 0.4).abs() < 1e-3 && (fx.v[1] + 0.3).abs() < 1e-6, "{:?}", fx.v);
    }

    #[test]
    fn old_sound_clips_become_sound_layers() {
        let mut doc = Document::default();
        let mut clip = crate::anim::AudioClip { id: 7, name: "bgm".into(), path: "bgm.wav".into(), start: 2.0, offset: 1.5, len: 3.0, gain: 0.5, mute: false, anim: Default::default() };
        clip.anim.keys.push(crate::anim::Key { t: 2.5, center: [0.0; 2], ext: [0.0; 2], rot: 0.0, opacity: 1.0, place: None, ease: Default::default(), follow: None, ch: crate::anim::ch::VOL, vol: 0.25, fx: [0.0; 4] });
        clips_to_layers(&mut doc, vec![clip]);
        let l = &doc.layers[0];
        let an = l.anim.as_ref().unwrap();
        assert_eq!(an.span, Some([2.0, 5.0]));
        assert_eq!(an.video_start, 1.5);
        assert_eq!(an.keys.len(), 1);
        assert!(matches!(&l.kind, LayerKind::Audio(a) if a.volume == 0.5 && a.path == "bgm.wav"));
        assert_eq!(l.name, "bgm");
    }
}
