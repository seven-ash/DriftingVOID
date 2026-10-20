//! Golden images of the CPU export: the render overhaul (docs/updates/
//! wgpu-renderer.md) must not change what an export looks like. Each scene is
//! rendered and compared with `tests/golden/<name>.png`; `DV_GOLDEN_WRITE=1`
//! writes them instead (only on purpose, after checking the new pictures).

use crate::doc::{Blend, Document, LayerKind, ShapeKind, ShapeLayer, TextLayer};
use crate::export::{render, render_ss};
use egui::{Color32, Rect, pos2, vec2};
use tiny_skia::Pixmap;

const ROOM_LINES: &str = "566.4,407.3,691.6,380.2,0;563.9,198.1,697.9,189.4,0;447.1,181.2,567.5,175.4,0;566.4,407.3,456.0,354.7,1;563.9,198.1,447.1,181.2,1;697.9,189.4,567.5,175.4,1;566.4,407.3,563.9,198.1,2;691.6,380.2,697.9,189.4,2;456.0,354.7,447.1,181.2,2";

fn text(pos: egui::Pos2, s: &str, size: f32, color: Color32) -> TextLayer {
    TextLayer { pos, text: s.into(), size, color, family: String::new(), style: String::new(), bg: None, wrap: 0.0, rotation: 0.0, blend: Blend::Normal, align: Default::default() }
}

fn shape(kind: ShapeKind, a: egui::Pos2, b: egui::Pos2, color: Color32, fill: Option<Color32>, rotation: f32) -> ShapeLayer {
    ShapeLayer { kind, a, b, color, width: 5.0, fill, rotation }
}

/// Flat 2D layers: shapes, text with opacity, rotation and blend modes, a brush
/// layer in multiply, a mask and an inverted mask at half strength.
fn flat_doc() -> Document {
    let mut doc = Document::default();
    doc.insert("bg".into(), LayerKind::Shape(shape(ShapeKind::Rect, pos2(0.0, 0.0), pos2(400.0, 300.0), Color32::from_rgb(30, 60, 140), Some(Color32::from_rgb(240, 200, 60)), 0.0)), None);
    doc.insert("e".into(), LayerKind::Shape(shape(ShapeKind::Ellipse, pos2(40.0, 30.0), pos2(220.0, 170.0), Color32::RED, Some(Color32::from_rgba_unmultiplied(40, 160, 220, 140)), 0.4)), None);
    let id = doc.insert("t1".into(), LayerKind::Text(text(pos2(30.0, 180.0), "Drifting 1", 40.0, Color32::from_rgb(20, 20, 20))), None);
    doc.get_mut(id).unwrap().opacity = 0.5;
    let mut t2 = text(pos2(200.0, 60.0), "곱하기", 52.0, Color32::from_rgb(120, 255, 120));
    t2.blend = Blend::Multiply;
    t2.rotation = -0.3;
    doc.insert("t2".into(), LayerKind::Text(t2), None);
    let mut t3 = text(pos2(220.0, 200.0), "Diff", 60.0, Color32::WHITE);
    t3.blend = Blend::Difference;
    doc.insert("t3".into(), LayerKind::Text(t3), None);
    let mut r = crate::raster::RasterLayer { blend: Blend::Multiply, ..Default::default() };
    r.fill_rect(Rect::from_min_size(pos2(300.0, 20.0), vec2(80.0, 260.0)), true);
    doc.insert("brush".into(), LayerKind::Raster(r), None);
    // a red square masked to its left half, and a green one with an inverted mask at half strength
    for (k, (col, inv, k_op)) in [(Color32::RED, false, 1.0), (Color32::GREEN, true, 0.5)].into_iter().enumerate() {
        let x = 60.0 + 120.0 * k as f32;
        let owner = doc.insert("sq".into(), LayerKind::Shape(shape(ShapeKind::Rect, pos2(x, 220.0), pos2(x + 100.0, 290.0), col, Some(col), 0.0)), None);
        let mut m = crate::raster::RasterLayer::default();
        m.fill_rect(Rect::from_min_size(pos2(x, 220.0), vec2(50.0, 70.0)), true);
        let mid = doc.insert("m".into(), LayerKind::Raster(m), None);
        let k0 = crate::anim::key_of(doc.get(owner).unwrap(), 0.0);
        let ml = doc.get_mut(mid).unwrap();
        ml.mask_of = Some(owner);
        ml.mask_anchor = Some((owner, k0.center));
        ml.mask_pose = Some([k0.rot, k0.ext[0]]);
        ml.mask_invert = inv;
        ml.opacity = k_op;
    }
    doc
}

/// The synthetic room photo with a solved space and light: a thick opaque text,
/// a thick translucent one (drawn as a group), a flat one with a blend mode, and
/// with `anim` the first one turning over two seconds on the timeline.
fn room_doc(anim: bool, t: f64) -> Document {
    let mut doc = crate::io::load_project(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/room.void"))).expect("room.void");
    let im = doc.layers.iter().find_map(|l| match &l.kind {
        LayerKind::Image(im) => Some(im.clone()),
        _ => None,
    }).unwrap();
    let sid = doc.new_id();
    let mut sp = crate::persp::Space::new(sid, "s".into(), Rect::from_points(&im.corners()));
    for part in ROOM_LINES.split(';') {
        let n: Vec<f32> = part.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        sp.lines.push(crate::persp::VLine { a: im.src_to_world(pos2(n[0], n[1])), b: im.src_to_world(pos2(n[2], n[3])), axis: n[4] as u8 });
    }
    let cam = crate::persp::solve(&sp).unwrap();
    sp.light = Some(crate::persp::Light::facing(&cam));
    doc.spaces.push(sp);
    let items: [(&str, f64, f32, Blend, Color32, egui::Vec2); 3] = [
        ("Drifting VOID", 30.0, 1.0, Blend::Normal, Color32::from_rgb(255, 214, 90), vec2(-150.0, 40.0)),
        ("반투명", 20.0, 0.6, Blend::Normal, Color32::from_rgb(90, 220, 255), vec2(-120.0, 120.0)),
        ("BLEND", 0.0, 1.0, Blend::Multiply, Color32::from_rgb(255, 90, 160), vec2(-60.0, -60.0)),
    ];
    for (k, (s, thick, op, blend, col, off)) in items.into_iter().enumerate() {
        let mut tx = text(im.center + off, s, 48.0, col);
        tx.blend = blend;
        let id = doc.insert("t".into(), LayerKind::Text(tx), None);
        let l = doc.get_mut(id).unwrap();
        l.opacity = op;
        let b = l.bounds().unwrap();
        let mut pl = crate::persp::Placement::facing(sid, &cam, b.center());
        pl.thick = thick;
        if k == 1 {
            pl.segs = 2;
            pl.smooth = true;
        }
        l.place = Some(pl);
        if anim && k == 0 {
            let k0 = crate::anim::key_of(l, 0.0);
            let mut k1 = k0;
            k1.t = 2.0;
            if let Some(p) = k1.place.as_mut() {
                p.rot[1] += 40.0;
            }
            l.anim = Some(crate::anim::Anim { keys: vec![k0, k1], ..Default::default() });
        }
    }
    if anim {
        doc.timeline.on = true;
        doc.timeline.time = t;
        crate::anim::evaluate(&mut doc);
    }
    doc
}

fn check(name: &str, pm: &Pixmap) {
    let path = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden")).join(format!("{name}.png"));
    let got = crate::export::to_rgba(pm);
    if std::env::var_os("DV_GOLDEN_WRITE").is_some() {
        got.save(&path).unwrap();
        return;
    }
    let want = image::open(&path).unwrap_or_else(|e| panic!("{}: {e} (DV_GOLDEN_WRITE=1 makes it)", path.display())).to_rgba8();
    assert_eq!(want.dimensions(), got.dimensions(), "{name}: size");
    // bands may differ by a level or two on anti-aliased edges (see
    // bands_match_a_single_pass); anything more is a change in the picture
    let n = (want.width() * want.height()) as usize;
    let (mut differing, mut worst, mut sum) = (0usize, 0u32, 0u64);
    for (a, b) in want.as_raw().chunks(4).zip(got.as_raw().chunks(4)) {
        let d = a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs()).max().unwrap();
        worst = worst.max(d);
        sum += d as u64;
        if d > 2 {
            differing += 1;
        }
    }
    if differing * 2000 > n || sum as f64 / n as f64 > 0.05 {
        let bad = path.with_extension("got.png");
        got.save(&bad).unwrap();
        panic!("{name}: {differing} of {n} pixels differ (worst {worst}, mean {:.3}); see {}", sum as f64 / n as f64, bad.display());
    }
}

#[test]
fn golden_flat() {
    let doc = flat_doc();
    let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
    check("flat", &render(&doc, region, 1.0, Some(Color32::WHITE)).unwrap());
    check("flat_ss2", &render_ss(&doc, region.center(), region.size(), 0.0, 1.0, Some(Color32::WHITE), 2).unwrap());
    // transparent background, rotated output, scaled
    check("flat_rot", &render_ss(&doc, region.center(), vec2(300.0, 200.0), 0.35, 1.5, None, 1).unwrap());
}

#[test]
fn golden_room() {
    let doc = room_doc(false, 0.0);
    let region = doc.content_bounds().unwrap();
    check("room", &render(&doc, region, 1.0, None).unwrap());
    check("room_ss2", &render_ss(&doc, region.center(), region.size(), 0.0, 0.75, None, 2).unwrap());
}

#[test]
fn golden_room_timeline() {
    let doc = room_doc(true, 1.0);
    let region = doc.content_bounds().unwrap();
    check("room_t1", &render(&doc, region, 1.0, None).unwrap());
}

/// Difference statistics between two pictures: share of pixels off by more than
/// 8 levels, mean difference, worst.
fn diff(a: &Pixmap, b: &Pixmap) -> (f64, f64, u32) {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    let n = (a.width() * a.height()) as f64;
    let (mut big, mut sum, mut worst) = (0usize, 0u64, 0u32);
    for (x, y) in a.data().chunks(4).zip(b.data().chunks(4)) {
        let d = x.iter().zip(y).map(|(p, q)| (*p as i32 - *q as i32).unsigned_abs()).max().unwrap();
        worst = worst.max(d);
        sum += d as u64;
        if d > 8 {
            big += 1;
        }
    }
    (big as f64 / n, sum as f64 / n, worst)
}

/// The GPU export against the CPU reference on the golden scenes (skipped
/// without a GPU). Tolerances: under 0.5 % of pixels off by more than 8 levels,
/// mean difference under 0.5 level (resampling and edge coverage differ a
/// little; anything structural fails).
#[test]
fn gpu_parity() {
    let cases: Vec<(&str, Document, egui::Pos2, egui::Vec2, f32, f32, Option<Color32>, u8)> = {
        let flat = flat_doc();
        let fr = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let room = room_doc(false, 0.0);
        let rr = room.content_bounds().unwrap();
        let room_t = room_doc(true, 1.0);
        vec![
            ("flat", flat.clone(), fr.center(), fr.size(), 0.0, 1.0, Some(Color32::WHITE), 1),
            ("flat_ss2", flat.clone(), fr.center(), fr.size(), 0.0, 1.0, Some(Color32::WHITE), 2),
            ("flat_rot", flat, fr.center(), vec2(300.0, 200.0), 0.35, 1.5, None, 1),
            ("room", room.clone(), rr.center(), rr.size(), 0.0, 1.0, None, 1),
            ("room_ss2", room, rr.center(), rr.size(), 0.0, 0.75, None, 2),
            ("room_t1", room_t, rr.center(), rr.size(), 0.0, 1.0, None, 1),
        ]
    };
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden"));
    let mut fails = Vec::new();
    for (name, doc, c, size, rot, scale, bg, k) in cases {
        let cpu = render_ss(&doc, c, size, rot, scale, bg, k).unwrap();
        let Some(gpu) = crate::gpu::with_gpu(|g| g.render_ss(&doc, c, size, rot, scale, bg, k)) else {
            eprintln!("no GPU: parity skipped");
            return;
        };
        let gpu = gpu.unwrap();
        crate::export::to_rgba(&gpu).save(dir.join(format!("{name}.gpu.got.png"))).unwrap();
        let (big, mean, worst) = diff(&cpu, &gpu);
        eprintln!("{name}: {:.3} % over 8 levels, mean {mean:.3}, worst {worst}", big * 100.0);
        if big > 0.005 || mean > 0.5 {
            fails.push(name);
        }
    }
    assert!(fails.is_empty(), "GPU differs from the CPU reference: {fails:?}");
}
