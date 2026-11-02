//! Golden images of the CPU export: the render overhaul (docs/updates/
//! wgpu-renderer.md) must not change what an export looks like. Each scene is
//! rendered and compared with `tests/golden/<name>.png`; `DV_GOLDEN_WRITE=1`
//! writes them instead (only on purpose, after checking the new pictures).

use crate::doc::{Blend, Document, LayerKind, ShapeKind, ShapeLayer, TextLayer};
use crate::export::{render, render_ss};
use egui::{Color32, Rect, pos2, vec2};
use tiny_skia::Pixmap;

pub const ROOM_LINES: &str = "566.4,407.3,691.6,380.2,0;563.9,198.1,697.9,189.4,0;447.1,181.2,567.5,175.4,0;566.4,407.3,456.0,354.7,1;563.9,198.1,447.1,181.2,1;697.9,189.4,567.5,175.4,1;566.4,407.3,563.9,198.1,2;691.6,380.2,697.9,189.4,2;456.0,354.7,447.1,181.2,2";

fn text(pos: egui::Pos2, s: &str, size: f32, color: Color32) -> TextLayer {
    TextLayer { pos, text: s.into(), size, color, family: String::new(), style: String::new(), bg: None, wrap: 0.0, rotation: 0.0, blend: Blend::Normal, align: Default::default(), look: Default::default() }
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

/// The room with its thick texts engraved (음각) `depth` deep, the opaque one
/// turned so its inner walls show (`thick`: None leaves the depth as it is).
fn engraved_doc(depth: Option<f64>) -> Document {
    let mut doc = room_doc(false, 0.0);
    for l in &mut doc.layers {
        if let Some(pl) = l.place.as_mut().filter(|p| p.thick > 0.0) {
            pl.engrave = true;
            if let Some(d) = depth {
                pl.thick = d;
            }
            if l.opacity >= 1.0 {
                pl.rot[1] += 30.0;
                pl.rot[0] -= 20.0;
            }
        }
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
/// little; anything structural fails). A photo shown small is resampled
/// differently (the CPU makes a smaller copy, the GPU blends mip levels): the
/// checkerboard's edges differ a little more there, under 4 % and 1.5 levels.
/// Text and shapes on the GPU are filled with exact area coverage, the CPU
/// supersamples shapes 4×4 and draws turned text as a resampled picture (the
/// GPU outlines it turned, sharper): the flat scenes allow 1.5 % / 0.5.
#[test]
fn gpu_parity() {
    let cases: Vec<(&str, Document, egui::Pos2, egui::Vec2, f32, f32, Option<Color32>, u8)> = {
        let flat = flat_doc();
        let fr = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let room = room_doc(false, 0.0);
        let rr = room.content_bounds().unwrap();
        let room_t = room_doc(true, 1.0);
        // the photo shown small (mip levels), and adjusted, keyed, cropped and turned
        let mut adj = room_doc(false, 0.0);
        for l in &mut adj.layers {
            if let LayerKind::Image(im) = &mut l.kind {
                im.adjust = crate::doc::Adjust { brightness: 0.15, contrast: 0.3, saturation: -0.4, chroma: Some([170, 100, 75]), ..Default::default() };
                im.crop = Rect::from_min_max(im.crop.min + vec2(37.0, 21.0), im.crop.max - vec2(50.0, 13.0));
                im.rotation = 0.2;
            }
        }
        // a copy of the photo, smaller and cropped, placed in the space and turned
        // (sampled straight from its texture on the GPU, flattened on the CPU)
        let mut p3 = room_doc(false, 0.0);
        let photo = p3.layers.iter().find(|l| matches!(l.kind, LayerKind::Image(_))).unwrap().clone();
        let sid = p3.spaces[0].id;
        let cam = crate::persp::solve(&p3.spaces[0]).unwrap();
        if let LayerKind::Image(mut im) = photo.kind {
            im.scale *= 0.35;
            im.crop = Rect::from_min_max(im.crop.min + vec2(20.0, 10.0), im.crop.max - vec2(30.0, 40.0));
            im.center += vec2(60.0, -30.0);
            let id = p3.insert("p".into(), LayerKind::Image(im), None);
            let l = p3.get_mut(id).unwrap();
            let mut pl = crate::persp::Placement::facing(sid, &cam, l.bounds().unwrap().center());
            pl.rot[1] += 35.0;
            pl.rot[0] -= 15.0;
            l.place = Some(pl);
        }
        let eng = engraved_doc(Some(60.0));
        let pm = photo_mask_doc().0;
        let pr = pm.content_bounds().unwrap();
        vec![
            ("room_photo_mask_small", pm, pr.center(), pr.size(), 0.0, 1.0, None, 1),
            ("room_engrave", eng.clone(), rr.center(), rr.size(), 0.0, 1.0, None, 1),
            ("room_engrave_ss2", eng, rr.center(), rr.size(), 0.0, 0.75, None, 2),
            ("room_photo3d", p3.clone(), rr.center(), rr.size(), 0.0, 1.0, None, 1),
            ("room_photo3d_small", p3, rr.center(), rr.size(), 0.0, 0.3, None, 1),
            ("room_small", room.clone(), rr.center(), rr.size(), 0.0, 0.2, None, 1),
            ("room_tiny_ss2", room.clone(), rr.center(), rr.size(), 0.0, 0.07, None, 2),
            ("room_adj", adj.clone(), rr.center(), rr.size(), 0.0, 1.0, None, 1),
            ("room_adj_small", adj, rr.center(), rr.size(), 0.0, 0.3, None, 1),
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
        crate::export::to_rgba(&cpu).save(dir.join(format!("{name}.cpu.got.png"))).unwrap();
        let (big, mean, worst) = diff(&cpu, &gpu);
        eprintln!("{name}: {:.3} % over 8 levels, mean {mean:.3}, worst {worst}", big * 100.0);
        let (big_max, mean_max) = if name.contains("small") || name.contains("tiny") {
            (0.04, 1.5)
        } else if name.starts_with("flat") {
            (0.015, 0.5)
        } else {
            (0.005, 0.5)
        };
        if big > big_max || mean > mean_max {
            fails.push(name);
        }
    }
    assert!(fails.is_empty(), "GPU differs from the CPU reference: {fails:?}");
}

/// An engraving draws only inside its shape on the plane: outside it the
/// picture is the room without the layer; inside, its cavity shows (the
/// floor and the inner walls, unlike the flat layer).
#[test]
fn engraving_stays_inside_its_opening() {
    let eng = engraved_doc(Some(80.0));
    let id = eng.layers.iter().find(|l| l.place.as_ref().is_some_and(|p| p.engrave) && l.opacity >= 1.0).unwrap().id;
    let mut none = eng.clone();
    none.layers.retain(|l| l.id != id);
    let mut flat = eng.clone();
    flat.get_mut(id).unwrap().place.as_mut().unwrap().thick = 0.0;
    let region = eng.content_bounds().unwrap();
    let (e, n, f) = (render(&eng, region, 1.0, None).unwrap(), render(&none, region, 1.0, None).unwrap(), render(&flat, region, 1.0, None).unwrap());
    let (w, h) = (e.width() as i32, e.height() as i32);
    let px = |p: &Pixmap, x: i32, y: i32| -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        p.data()[i..i + 4].try_into().unwrap()
    };
    let near_shape = |x: i32, y: i32| (-2..=2).any(|dy| (-2..=2).any(|dx| {
        let (xx, yy) = (x + dx, y + dy);
        xx >= 0 && yy >= 0 && xx < w && yy < h && px(&f, xx, yy) != px(&n, xx, yy)
    }));
    let (mut outside_changed, mut inside_differs_from_flat) = (0, 0);
    for y in 0..h {
        for x in 0..w {
            if near_shape(x, y) {
                inside_differs_from_flat += (px(&e, x, y) != px(&f, x, y)) as u32;
            } else if px(&e, x, y) != px(&n, x, y) {
                outside_changed += 1;
            }
        }
    }
    assert_eq!(outside_changed, 0, "pixels changed outside the opening");
    assert!(inside_differs_from_flat > 500, "the cavity shows ({inside_differs_from_flat} pixels differ from the flat text)");
}

/// Close-up of the turned text engraved and extruded side by side, 4× (for
/// looking: `DV_ENGRAVE_PNG=path.png cargo test --release engrave_closeup
/// -- --ignored`; `DV_ENGRAVE_DEPTH`, default 12).
#[test]
#[ignore]
fn engrave_closeup() {
    let depth: f64 = std::env::var("DV_ENGRAVE_DEPTH").ok().and_then(|v| v.parse().ok()).unwrap_or(12.0);
    let eng = engraved_doc(Some(depth));
    let mut ext = eng.clone();
    for l in &mut ext.layers {
        if let Some(p) = l.place.as_mut() {
            p.engrave = false;
        }
    }
    let l = eng.layers.iter().find(|l| l.place.as_ref().is_some_and(|p| p.engrave) && l.opacity >= 1.0).unwrap();
    let region = l.shown_bounds(&crate::persp::cameras(&eng.spaces)).unwrap().expand(10.0);
    let (a, b) = (render(&eng, region, 4.0, None).unwrap(), render(&ext, region, 4.0, None).unwrap());
    let mut out = image::RgbaImage::new(a.width(), a.height() * 2 + 8);
    image::imageops::replace(&mut out, &crate::export::to_rgba(&a), 0, 0);
    image::imageops::replace(&mut out, &crate::export::to_rgba(&b), 0, (a.height() + 8) as i64);
    if let Ok(p) = std::env::var("DV_ENGRAVE_PNG") {
        out.save(p).unwrap();
    }
}

/// The room photo shown at 40 %, turned 0.2 rad, with a text over it whose
/// mask is a photo mask (its pixels are the photo's): a rectangle of source
/// pixels (300..520 × 180..420, the cube) hidden.
fn photo_mask_doc() -> (Document, crate::doc::LayerId, crate::doc::LayerId) {
    let mut doc = crate::io::load_project(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/room.void"))).expect("room.void");
    let pid = doc.layers.iter().find(|l| matches!(l.kind, LayerKind::Image(_))).unwrap().id;
    let (cc, centre) = {
        let LayerKind::Image(im) = &mut doc.get_mut(pid).unwrap().kind else { unreachable!() };
        im.scale = 0.4;
        im.rotation = 0.2;
        (im.crop.center(), im.center)
    };
    let tx = doc.insert("t".into(), LayerKind::Text(text(centre - vec2(220.0, 60.0), "MASKED", 90.0, Color32::from_rgb(255, 214, 90))), None);
    let mut m = crate::raster::RasterLayer::default();
    m.fill_rect(Rect::from_min_max(pos2(300.0, 180.0), pos2(520.0, 420.0)), true);
    let mid = doc.insert("m".into(), LayerKind::Raster(m), None);
    let ml = doc.get_mut(mid).unwrap();
    ml.mask_of = Some(tx);
    ml.mask_anchor = Some((pid, [cc.x, cc.y]));
    ml.mask_pose = Some([0.0, 0.4]);
    ml.mask_res = 1.0 / 0.4;
    (doc, pid, mid)
}

/// A photo mask keeps to its photo's source pixels: the same source points
/// are hidden before and after the photo is moved, turned and resized, and
/// the mask's pixels are never touched.
#[test]
fn photo_mask_follows_its_photo_pixel_for_pixel() {
    let (mut doc, pid, mid) = photo_mask_doc();
    let hidden = |doc: &Document, s: egui::Pos2| {
        let LayerKind::Image(im) = &doc.get(pid).unwrap().kind else { unreachable!() };
        let m = doc.get(mid).unwrap();
        crate::export::mask_hide(m, crate::export::mask_map(doc, m), im.src_to_world(s))
    };
    let inside = [pos2(301.0, 181.0), pos2(410.0, 300.0), pos2(519.0, 419.0)];
    let outside = [pos2(299.0, 300.0), pos2(410.0, 421.0), pos2(600.0, 100.0)];
    let tiles = match &doc.get(mid).unwrap().kind {
        LayerKind::Raster(r) => r.tiles.len(),
        _ => 0,
    };
    for step in 0..3 {
        for s in inside {
            assert_eq!(hidden(&doc, s), 1.0, "step {step}: {s:?} should be hidden");
        }
        for s in outside {
            assert_eq!(hidden(&doc, s), 0.0, "step {step}: {s:?} should show");
        }
        let LayerKind::Image(im) = &mut doc.get_mut(pid).unwrap().kind else { unreachable!() };
        im.scale *= [2.3, 0.37, 1.0][step];
        im.rotation -= 0.55;
        im.center += vec2(37.5, -12.25);
        crate::app::sync_masks(&mut doc, false);
    }
    let LayerKind::Raster(r) = &doc.get(mid).unwrap().kind else { unreachable!() };
    assert_eq!(r.tiles.len(), tiles, "the mask was not resampled");
}

/// A pen layer of `n` random strokes of 150 points over 1600×1000 (`seed`),
/// some translucent.
fn pen_doc(n: usize, seed: u32) -> Document {
    let mut strokes = Vec::new();
    let mut seed = seed.wrapping_mul(2654435761).wrapping_add(7);
    let mut rnd = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    for k in 0..n {
        let (mut x, mut y) = (rnd() * 1600.0, rnd() * 1000.0);
        let (mut dx, mut dy) = (rnd() - 0.5, rnd() - 0.5);
        let mut pts = Vec::new();
        for i in 0..150 {
            dx += (rnd() - 0.5) * 0.4;
            dy += (rnd() - 0.5) * 0.4;
            x += dx * 4.0;
            y += dy * 4.0;
            pts.push([x, y, 0.4 + 6.0 * (i as f32 / 150.0 * 3.14).sin()]);
        }
        let col = match k % 3 {
            0 => Color32::from_rgb(30, 60, 200),
            1 => Color32::from_rgba_unmultiplied(220, 60, 40, 140),
            _ => Color32::from_rgb(20, 20, 20),
        };
        strokes.push(std::sync::Arc::new(crate::vector::VStroke::new(pts, col)));
    }
    // a dot (one point)
    strokes.push(std::sync::Arc::new(crate::vector::VStroke::new(vec![[800.0, 500.0, 9.0]], Color32::from_rgb(0, 160, 90))));
    let mut doc = Document::default();
    doc.insert("v".into(), LayerKind::Vector(crate::doc::VectorLayer { offset: vec2(3.5, -2.25), strokes }), None);
    doc
}

/// Pen strokes on the GPU (largest capsule coverage per stroke) against the
/// CPU's filled capsule outlines, at three scales. Measured: 2.4 % / 0.57 at
/// 0.37; see the test output for the others.
#[test]
fn gpu_pen_matches_cpu() {
    let doc = pen_doc(60, 3);
    let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 1000.0));
    for s in [0.37f32, 1.0, 2.5] {
        let cpu = render_ss(&doc, region.center(), region.size(), 0.0, s, Some(Color32::WHITE), 1).unwrap();
        let Some(gpu) = crate::gpu::with_gpu(|g| g.render_ss(&doc, region.center(), region.size(), 0.0, s, Some(Color32::WHITE), 1)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        let gpu = gpu.unwrap();
        let (big, mean, worst) = diff(&cpu, &gpu);
        eprintln!("pen at {s}: {:.3} % over 8 levels, mean {mean:.3}, worst {worst}", big * 100.0);
        if std::env::var_os("DV_SAVE_PEN").is_some() {
            let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden"));
            crate::export::to_rgba(&gpu).save(dir.join(format!("pen_{s}.gpu.got.png"))).unwrap();
            crate::export::to_rgba(&cpu).save(dir.join(format!("pen_{s}.cpu.got.png"))).unwrap();
        }
        // shown small, the CPU's capsule outlines are coarse polygons: edge
        // pixels differ more there (the small-photo tolerance)
        let (big_max, mean_max) = if s < 0.5 { (0.04, 1.5) } else { (0.015, 0.5) };
        assert!(big < big_max && mean < mean_max, "pen at {s}: {:.3} %, mean {mean:.3}", big * 100.0);
    }
}

/// What is left on the CPU, timed (render-finish items 3–5): a pen layer
/// rasterized, a thick text flattened and traced, big brush dabs.
/// `cargo test --release bench_cpu_work -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_cpu_work() {
    use std::time::Instant;
    // 3. a pen layer: 400 strokes of 150 points over 1600×1000
    let doc = pen_doc(400, 1);
    let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 1000.0));
    for s in [0.5f32, 1.0, 2.0] {
        let t = Instant::now();
        let _ = render(&doc, region, s, None).unwrap();
        eprintln!("pen layer, 400 strokes × 150 points, at {s}: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let _ = crate::gpu::with_gpu(|g| {
            g.clear();
            let t = Instant::now();
            let _ = g.render_ss(&doc, region.center(), region.size(), 0.0, s, None, 1);
            eprintln!("  on the GPU (with read back): {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        });
    }
    // 4. a thick text flattened and its outline traced (cold cache)
    let room = room_doc(false, 0.0);
    let l = room.layers.iter().find(|l| l.place.as_ref().is_some_and(|p| p.thick > 0.0)).unwrap();
    let pl = l.place.clone().unwrap();
    for res in [1.0f32, 2.0, 4.0] {
        let fc = crate::export::FlatCache::default();
        let t = Instant::now();
        let f = fc.get(l, &pl, res);
        eprintln!("thick text flatten + outline at {res}: {:.1} ms ({} edges)", t.elapsed().as_secs_f64() * 1e3, f.and_then(|f| f.3.clone()).map_or(0, |e| e.len()));
    }
    // 5. brush dabs: soft radius 200 and 40, points 4 px apart (a quick pen move)
    for (r, hard) in [(200.0f32, 0.2f32), (40.0, 0.8)] {
        let mut layer = crate::raster::RasterLayer::default();
        let mut st = crate::raster::RasterStroke::new(crate::raster::BrushParams { color: Color32::RED, radius: r, opacity: 1.0, hardness: hard, pressure_size: false, pressure_opacity: false, erase: false });
        let t = Instant::now();
        let n = 200;
        for i in 0..n {
            st.add_point(&mut layer, pos2(100.0 + i as f32 * 4.0, 300.0 + (i as f32 * 0.05).sin() * 80.0), 1.0);
        }
        st.finish(&mut layer);
        eprintln!("brush radius {r}: {:.2} ms per pen point ({n} points)", t.elapsed().as_secs_f64() * 1e3 / n as f64);
    }
}

/// The loupe, the eyedropper and copying draw on the canvas's renderer
/// between its frames (`render_aside`): small regions at a high scale and a
/// single pixel must match the CPU, and drawing aside must leave the caches
/// as they were (a second picture reuses them).
#[test]
fn aside_matches_cpu() {
    let doc = room_doc(false, 0.0);
    let im = doc.layers.iter().find_map(|l| match &l.kind {
        LayerKind::Image(im) => Some(im.clone()),
        _ => None,
    }).unwrap();
    let c = im.center;
    let regions = [(Rect::from_center_size(c, vec2(24.0, 24.0)), 6.0), (Rect::from_center_size(c + vec2(-150.0, 40.0), vec2(40.0, 30.0)), 4.0), (Rect::from_min_size(c.floor(), vec2(1.0, 1.0)), 1.0)];
    let r = crate::gpu::with_gpu(|g| {
        let mut out = Vec::new();
        for (reg, sc) in regions {
            let cpu = render(&doc, reg, sc, None).unwrap();
            let gpu = g.render_aside(&doc, reg, sc, None).unwrap();
            out.push(diff(&cpu, &gpu));
        }
        out
    });
    let Some(r) = r else {
        eprintln!("no GPU: skipped");
        return;
    };
    for (i, (big, mean, worst)) in r.into_iter().enumerate() {
        eprintln!("region {i}: {:.3} % over 8 levels, mean {mean:.3}, worst {worst}", big * 100.0);
        assert!(big < 0.01 && mean < 1.0, "region {i} differs");
    }
}

/// Upright text filled on the GPU from its outlines matches ab_glyph's own
/// rasterizing closely (the same area coverage), at several sizes and
/// sub-pixel positions, with a background. What differs is the curves: ab_glyph
/// flattens them coarsely at small sizes, the GPU's outlines are finer (same
/// ink to 1 %, centres within 0.1 px).
#[test]
fn gpu_text_matches_cpu() {
    let mut doc = Document::default();
    for (k, (sz, x)) in [(13.0, 3.3), (31.0, 7.71), (72.0, 0.5)].into_iter().enumerate() {
        let mut t = text(pos2(x, 10.0 + 90.0 * k as f32), "Drifting gÅ 곱하기", sz, Color32::from_rgb(20, 40, 200));
        if k == 1 {
            t.bg = Some(Color32::from_rgba_unmultiplied(250, 200, 60, 200));
        }
        doc.insert("t".into(), LayerKind::Text(t), None);
    }
    // box options and the letter outline, turned
    let mut t = text(pos2(380.0, 120.0), "Outline 외곽", 44.0, Color32::from_rgb(255, 230, 90));
    t.bg = Some(Color32::from_rgb(40, 60, 90));
    t.look = crate::doc::TextLook { pad_x: 0.6, pad_y: 0.2, radius: 0.5, border: Some(Color32::from_rgb(230, 60, 60)), border_w: 0.08, stroke: Some(Color32::BLACK), stroke_w: 0.1 };
    doc.insert("t".into(), LayerKind::Text(t), None);
    let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 300.0));
    for scale in [1.0, 2.37] {
        let cpu = render(&doc, region, scale, Some(Color32::WHITE)).unwrap();
        let Some(gpu) = crate::gpu::with_gpu(|g| g.render_ss(&doc, region.center(), region.size(), 0.0, scale, Some(Color32::WHITE), 1)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        let gpu = gpu.unwrap();
        let (big, mean, worst) = diff(&cpu, &gpu);
        eprintln!("text ×{scale}: {:.3} % over 8 levels, mean {mean:.3}, worst {worst}", big * 100.0);
        assert!(big < 0.015 && mean < 0.3, "text differs at ×{scale}");
    }
}

/// A mask that hides nothing (empty, or painted far away) leaves a thick 3D
/// text exactly as it is without one (Ash, 10-21: an empty mask cut the
/// sides).
#[test]
fn empty_mask_leaves_thick_text() {
    let mut plain = room_doc(false, 0.0);
    let owner = plain.layers.iter().find(|l| l.place.as_ref().is_some_and(|p| p.thick > 0.0) && l.opacity >= 1.0).unwrap().id;
    // deep: the sides reach well past the front face's bounds
    plain.get_mut(owner).unwrap().place.as_mut().unwrap().thick = 400.0;
    let region = plain.content_bounds().unwrap();
    let with_mask = |paint: bool| {
        let mut d = plain.clone();
        let mut m = crate::raster::RasterLayer::default();
        if paint {
            m.fill_rect(Rect::from_min_size(region.min + vec2(2.0, 2.0), vec2(4.0, 4.0)), true);
        }
        let mid = d.insert("m".into(), LayerKind::Raster(m), None);
        let k0 = crate::anim::key_of(d.get(owner).unwrap(), 0.0);
        let ml = d.get_mut(mid).unwrap();
        ml.mask_of = Some(owner);
        ml.mask_anchor = Some((owner, k0.center));
        ml.mask_invert = false;
        d
    };
    let r = crate::gpu::with_gpu(|g| {
        let draw = |g: &mut crate::gpu::Gpu, d: &Document| g.render_ss(d, region.center(), region.size(), 0.0, 1.0, Some(Color32::BLACK), 1).unwrap();
        let a = draw(g, &plain);
        let b = draw(g, &with_mask(false));
        let c = draw(g, &with_mask(true));
        (diff(&a, &b), diff(&a, &c))
    });
    let Some((empty, far)) = r else {
        eprintln!("no GPU: skipped");
        return;
    };
    eprintln!("empty {empty:?}, far {far:?}");
    assert!(empty.0 < 1e-4 && far.0 < 1e-4, "a mask that hides nothing changed the picture");
}

/// DV_WRITE_FLAT=path.void: saves the flat golden scene (brush layer, masks,
/// blend modes) as a project, to open it in the app for canvas captures.
#[test]
#[ignore]
fn write_flat_void() {
    if let Some(p) = std::env::var_os("DV_WRITE_FLAT") {
        let mut doc = flat_doc();
        // a brush layer that shows (red strokes over the left)
        let mut r = crate::raster::RasterLayer::default();
        r.fill_rect(Rect::from_min_size(pos2(10.0, 10.0), vec2(30.0, 280.0)), true);
        doc.insert("brush2".into(), LayerKind::Raster(r), None);
        crate::io::save_project(&doc, std::path::Path::new(&p)).unwrap();
    }
    // DV_WRITE_TILES: a brush layer of 400 tiles (20×20, 5120 px square, a
    // stroke pattern in each) for frame-time checks
    if let Some(p) = std::env::var_os("DV_WRITE_TILES") {
        let mut r = crate::raster::RasterLayer::default();
        for k in 0..20 {
            let y = k as f32 * 256.0 + 40.0;
            r.fill_rect(Rect::from_min_max(pos2(0.0, y), pos2(5120.0, y + 30.0)), true);
            let x = k as f32 * 256.0 + 100.0;
            r.fill_rect(Rect::from_min_max(pos2(x, 0.0), pos2(x + 30.0, 5120.0)), true);
        }
        let mut doc = Document::default();
        doc.insert("tiles".into(), LayerKind::Raster(r), None);
        crate::io::save_project(&doc, std::path::Path::new(&p)).unwrap();
    }
    // DV_WRITE_PEN: the 400-stroke pen layer of `bench_cpu_work` (canvas sweeps)
    if let Some(p) = std::env::var_os("DV_WRITE_PEN") {
        crate::io::save_project(&pen_doc(400, 1), std::path::Path::new(&p)).unwrap();
    }
}

/// DV_INSPECT=path.void: lists a project's layers and how the scene draws them.
#[test]
#[ignore]
fn inspect_project() {
    let Some(p) = std::env::var_os("DV_INSPECT") else { return };
    let doc = crate::io::load_project(std::path::Path::new(&p)).unwrap();
    let cams = crate::persp::cameras(&doc.spaces);
    for (i, l) in doc.layers.iter().enumerate() {
        let kind = match &l.kind {
            LayerKind::Text(t) => format!("text {:?} blend {:?} size {}", t.text, t.blend, t.size),
            LayerKind::Raster(r) => format!("brush blend {:?}", r.blend),
            LayerKind::Image(im) => format!("image video {}", im.video.is_some()),
            k => format!("{:?}", std::mem::discriminant(k)),
        };
        let pl = l.place.as_ref().map(|p| format!("space {} thick {} segs {} smooth {} rest {:?}", p.space, p.thick, p.segs, p.smooth, p.rest));
        eprintln!("{i:2} id {} {:?} vis {} op {} mask_of {:?} | {kind} | {pl:?} | span {:?}", l.id, l.name, l.visible, l.opacity, l.mask_of, l.anim.as_ref().and_then(|a| a.span));
    }
    eprintln!("draw order:");
    for l in doc.draw_order(&cams) {
        eprintln!("  {} {:?}", l.id, l.name);
    }
    let scene = crate::scene::build(&doc);
    for it in &scene.items {
        let what = match &it.draw {
            crate::scene::Draw::Flat(l) => format!("Flat {:?}", l.name),
            crate::scene::Draw::Blend(l, b) => format!("Blend {:?} {b:?}", l.name),
            crate::scene::Draw::Group { mode, opacity, inner } => format!("Group {mode:?} {opacity} inner {}", match inner.as_ref() { crate::scene::Draw::Projected(p) => format!("Projected {:?}", p.layer.name), crate::scene::Draw::Flat(l) => format!("Flat {:?}", l.name), _ => "other".into() }),
            crate::scene::Draw::Masked { .. } => "Masked".into(),
            crate::scene::Draw::Fx { fx, .. } => format!("Fx {:?}", fx.filter),
            crate::scene::Draw::Projected(p) => format!("Projected {:?} thick {}", p.layer.name, p.pl.thick),
        };
        eprintln!("  item {what} bounds {:?}", it.bounds);
    }
}

/// A masked layer with a blend mode mixes with what is below it (the mask cuts
/// it first), on the CPU export and the GPU: a white brush layer in difference
/// over red shows cyan where the mask keeps it, red where it hides it.
#[test]
fn masked_layer_keeps_its_blend_mode() {
    let mut doc = Document::default();
    doc.insert("bg".into(), LayerKind::Shape(shape(ShapeKind::Rect, pos2(0.0, 0.0), pos2(100.0, 100.0), Color32::RED, Some(Color32::RED), 0.0)), None);
    let mut r = crate::raster::RasterLayer { blend: Blend::Difference, ..Default::default() };
    r.fill_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0)), true);
    let owner = doc.insert("w".into(), LayerKind::Raster(r), None);
    let mut m = crate::raster::RasterLayer::default();
    m.fill_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(50.0, 100.0)), true);
    let mid = doc.insert("m".into(), LayerKind::Raster(m), None);
    doc.get_mut(mid).unwrap().mask_of = Some(owner);
    let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0));
    let check = |pm: &Pixmap, what: &str| {
        let (hid, kept) = (pm.pixel(25, 50).unwrap(), pm.pixel(75, 50).unwrap());
        assert!(hid.red() > 240 && hid.green() < 15 && hid.blue() < 15, "{what}: hidden half {hid:?}");
        assert!(kept.red() < 15 && kept.green() > 240 && kept.blue() > 240, "{what}: kept half {kept:?}");
    };
    check(&render(&doc, region, 1.0, Some(Color32::BLACK)).unwrap(), "CPU");
    if let Some(Ok(pm)) = crate::gpu::with_gpu(|g| g.render_ss(&doc, region.center(), region.size(), 0.0, 1.0, Some(Color32::BLACK), 1)) {
        check(&pm, "GPU");
    }
}

/// FX layers on the GPU (fx-layers.md): a blur spreads a bar's edge both
/// ways in 복사 (the picture below is replaced, alpha too) but keeps the
/// opaque original under it in 보통; a mask keeps the hidden half as it was;
/// brightness darkens. Skipped without a GPU.
#[test]
fn fx_layers() {
    use crate::doc::{Filter, FxLayer};
    // the bar covers x 37.5..62.5 (the shape's own margin)
    let bar = |doc: &mut Document| {
        doc.insert("bar".into(), LayerKind::Shape(shape(ShapeKind::Rect, pos2(40.0, 0.0), pos2(60.0, 100.0), Color32::RED, Some(Color32::RED), 0.0)), None);
    };
    let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0));
    let draw = |doc: &Document| crate::gpu::with_gpu(|g| g.render_ss(doc, region.center(), region.size(), 0.0, 1.0, None, 1)).and_then(|r| r.ok());
    let blur = |blend| LayerKind::Fx(FxLayer { v: [10.0, 0.0, 0.0, 0.0], blend, ..FxLayer::new(Filter::Blur) });

    let mut doc = Document::default();
    bar(&mut doc);
    doc.insert("fx".into(), blur(None), None);
    let Some(pm) = draw(&doc) else { return };
    let (inside, outside) = (pm.pixel(40, 50).unwrap(), pm.pixel(66, 50).unwrap());
    assert!(inside.alpha() < 250, "복사: the edge inside softens {inside:?}");
    assert!(outside.alpha() > 10, "복사: the edge spreads out {outside:?}");
    assert!(pm.pixel(50, 50).unwrap().red() > 200);

    // the same canvas point alike whatever the frame: with the bar's edge at
    // the frame's left edge the blur still sees the bar past it
    let shifted = crate::gpu::with_gpu(|g| g.render_ss(&doc, pos2(110.0, 50.0), vec2(100.0, 100.0), 0.0, 1.0, None, 1)).and_then(|r| r.ok()).unwrap();
    for x in [61, 63, 66, 70] {
        let (a, b) = (pm.pixel(x, 50).unwrap().alpha() as i32, shifted.pixel(x - 60, 50).unwrap().alpha() as i32);
        assert!((a - b).abs() <= 3, "x {x}: {a} vs {b} in a frame starting at 60");
    }

    let mut doc = Document::default();
    bar(&mut doc);
    doc.insert("fx".into(), blur(Some(Blend::Normal)), None);
    let pm = draw(&doc).unwrap();
    assert!(pm.pixel(40, 50).unwrap().alpha() > 250, "보통: the original stays under the blur");
    assert!(pm.pixel(66, 50).unwrap().alpha() > 10);

    // a mask hiding the left half: the left edge stays sharp, the right spreads
    let mut doc = Document::default();
    bar(&mut doc);
    let fx = doc.insert("fx".into(), blur(None), None);
    let mut m = crate::raster::RasterLayer::default();
    m.fill_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(50.0, 100.0)), true);
    let mid = doc.insert("m".into(), LayerKind::Raster(m), None);
    doc.get_mut(mid).unwrap().mask_of = Some(fx);
    let pm = draw(&doc).unwrap();
    assert_eq!(pm.pixel(34, 50).unwrap().alpha(), 0, "masked side as it was");
    assert!(pm.pixel(40, 50).unwrap().alpha() > 250);
    assert!(pm.pixel(66, 50).unwrap().alpha() > 10, "open side blurred");

    let mut doc = Document::default();
    bar(&mut doc);
    doc.insert("fx".into(), LayerKind::Fx(FxLayer { v: [-0.5, 0.0, 0.0, 0.0], ..FxLayer::new(Filter::BrightContrast) }), None);
    let p = draw(&doc).unwrap().pixel(50, 50).unwrap();
    assert!((110..=145).contains(&p.red()) && p.alpha() == 255, "brightness -0.5 {p:?}");
}

/// The colour FX filters on the GPU against values computed independently
/// (Python's colorsys and the same published formulas) for one colour,
/// 200·100·50. Skipped without a GPU.
#[test]
fn fx_colour_filters() {
    use crate::doc::{Filter, FxLayer};
    let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(40.0, 40.0));
    let px = |base: Color32, fx: FxLayer| {
        let mut doc = Document::default();
        doc.insert("c".into(), LayerKind::Shape(shape(ShapeKind::Rect, pos2(-10.0, -10.0), pos2(50.0, 50.0), base, Some(base), 0.0)), None);
        doc.insert("fx".into(), LayerKind::Fx(fx), None);
        crate::gpu::with_gpu(|g| g.render_ss(&doc, region.center(), region.size(), 0.0, 1.0, None, 1)).and_then(|r| r.ok()).map(|pm| {
            let p = pm.pixel(20, 20).unwrap();
            [p.red(), p.green(), p.blue()]
        })
    };
    let c = Color32::from_rgb(200, 100, 50);
    let fx = |f: Filter, v: [f32; 4]| FxLayer { v, ..FxLayer::new(f) };
    let mut grad = FxLayer::new(Filter::GradientMap);
    grad.colors = [[20, 40, 120, 255], [250, 230, 180, 255]];
    let cases: Vec<(&str, FxLayer, [u8; 3])> = vec![
        ("밝기/대비", fx(Filter::BrightContrast, [0.2, 0.3, 0.0, 0.0]), [255, 133, 68]),
        ("색조/채도/명도", fx(Filter::Hsl, [120.0, -0.5, 0.2, 0.0]), [120, 182, 141]),
        ("레벨", fx(Filter::Levels, [0.1, 2.0, 0.9, 0.2]), [240, 174, 122]),
        ("흑백", fx(Filter::Desaturate, [0.5, 0.0, 0.0, 0.0]), [159, 109, 84]),
        ("반전", fx(Filter::Invert, [0.0; 4]), [55, 155, 205]),
        ("포스터화", fx(Filter::Posterize, [4.0, 0.0, 0.0, 0.0]), [170, 85, 85]),
        ("임계값", fx(Filter::Threshold, [0.3, 0.0, 0.0, 0.0]), [255, 255, 255]),
        ("그라디언트 맵", grad, [126, 128, 148]),
        ("커브 (그대로)", FxLayer::new(Filter::Curves), [200, 100, 50]),
        // red's own curve halves it, then the RGB curve inverts every channel
        // (each channel's curve first, then RGB, as GIMP)
        ("커브 (R, RGB)", curves(&[[0.0, 1.0], [1.0, 0.0]], &[[0.0, 0.0], [1.0, 0.5]], 1.0), [155, 155, 205]),
        // 양 0.5: halfway between the original and that
        ("커브 (양)", curves(&[[0.0, 1.0], [1.0, 0.0]], &[[0.0, 0.0], [1.0, 0.5]], 0.5), [178, 128, 128]),
    ];
    for (name, f, want) in cases {
        let Some(got) = px(c, f) else { return };
        for i in 0..3 {
            assert!((got[i] as i32 - want[i] as i32).abs() <= 2, "{name}: got {got:?}, want {want:?}");
        }
    }
    // a curve through its middle point: grey 128 (x 0.502) to about 0.7
    let s = [[0.0, 0.0], [0.25, 0.4], [0.5, 0.7], [0.75, 0.9], [1.0, 1.0]];
    let got = px(Color32::from_gray(128), curves(&s, &[[0.0, 0.0], [1.0, 1.0]], 1.0)).unwrap();
    assert!((got[0] as i32 - 179).abs() <= 3, "커브: {got:?}");

    // 채널을 알파로: the channel (of the picture over black) through the
    // curve becomes the alpha; 양 mixes in the old one. Alpha and
    // premultiplied red: R 200 makes alpha 200 (red 200 × 200 / 255)
    let alpha = |source: u8, curve: &[[f32; 2]], amount: f32| {
        let mut f = FxLayer::new(Filter::ChannelAlpha);
        f.source = source;
        f.curves[0] = crate::doc::curve_pts(curve);
        f.v[0] = amount;
        let mut doc = Document::default();
        doc.insert("c".into(), LayerKind::Shape(shape(ShapeKind::Rect, pos2(-10.0, -10.0), pos2(50.0, 50.0), c, Some(c), 0.0)), None);
        doc.insert("fx".into(), LayerKind::Fx(f), None);
        crate::gpu::with_gpu(|g| g.render_ss(&doc, region.center(), region.size(), 0.0, 1.0, None, 1)).and_then(|r| r.ok()).map(|pm| {
            let p = pm.pixel(20, 20).unwrap();
            [p.alpha(), p.red()]
        })
    };
    let id = [[0.0, 0.0], [1.0, 1.0]];
    for (name, source, curve, amount, want) in [
        ("R", 0, &id[..], 1.0, [200, 157]),
        ("G", 1, &id[..], 1.0, [100, 78]),
        // Rec.709 luma of (200, 100, 50): 0.461
        ("L", 3, &id[..], 1.0, [118, 92]),
        ("R, 뒤집은 커브", 0, &[[0.0, 1.0], [1.0, 0.0]][..], 1.0, [55, 43]),
        ("R, 양 0.5", 0, &id[..], 0.5, [227, 178]),
    ] {
        let Some(got) = alpha(source, curve, amount) else { return };
        assert!(got.iter().zip(want).all(|(g, w)| (*g as i32 - w).abs() <= 2), "채널을 알파로 ({name}): got {got:?}, want {want:?}");
    }
}

/// A 커브 layer with these RGB and red curves at this 양.
fn curves(rgb: &[[f32; 2]], red: &[[f32; 2]], amount: f32) -> crate::doc::FxLayer {
    let mut f = crate::doc::FxLayer::new(crate::doc::Filter::Curves);
    f.curves[0] = crate::doc::curve_pts(rgb);
    f.curves[1] = crate::doc::curve_pts(red);
    f.v[0] = amount;
    f
}

/// Motion blur and unsharp mask against their definitions, measured from the
/// unfiltered edge (where it crosses half) so the shapes' own antialiasing
/// does not matter. Skipped without a GPU.
#[test]
fn fx_blur_sharpen() {
    use crate::doc::{Filter, FxLayer};
    let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0));
    let draw = |doc: &Document| crate::gpu::with_gpu(|g| g.render_ss(doc, region.center(), region.size(), 0.0, 1.0, None, 1)).and_then(|r| r.ok());
    let edge_doc = || {
        let mut doc = Document::default();
        doc.insert("left".into(), LayerKind::Shape(shape(ShapeKind::Rect, pos2(-200.0, -200.0), pos2(50.0, 300.0), Color32::RED, Some(Color32::RED), 0.0)), None);
        doc
    };
    let Some(plain) = draw(&edge_doc()) else { return };
    // the half-crossing of the vertical edge, sub-pixel (row 50)
    let a = |pm: &Pixmap, x: u32| pm.pixel(x, 50).unwrap().alpha() as f32 / 255.0;
    let x0 = (1..99).find(|&x| a(&plain, x) < 0.5).unwrap();
    let e0 = (x0 - 1) as f32 + 0.5 + (a(&plain, x0 - 1) - 0.5) / (a(&plain, x0 - 1) - a(&plain, x0));
    let at = |pm: &Pixmap, x: f32| {
        let i = (x - 0.5).floor();
        let f = x - 0.5 - i;
        a(pm, i as u32) * (1.0 - f) + a(pm, i as u32 + 1) * f
    };
    let mut doc = edge_doc();
    doc.insert("fx".into(), LayerKind::Fx(FxLayer { v: [20.0, 0.0, 0.0, 0.0], ..FxLayer::new(Filter::MotionBlur) }), None);
    let pm = draw(&doc).unwrap();
    for (dx, want) in [(-5.0, 0.75), (0.0, 0.5), (5.0, 0.25), (-12.0, 1.0), (12.0, 0.0)] {
        let got = at(&pm, e0 + dx);
        assert!((got - want).abs() <= 0.04, "동작 흐림 0°: alpha at edge{dx:+} {got:.3}, want {want}");
    }
    let mut doc = edge_doc();
    doc.insert("fx".into(), LayerKind::Fx(FxLayer { v: [20.0, 90.0, 0.0, 0.0], ..FxLayer::new(Filter::MotionBlur) }), None);
    let pm = draw(&doc).unwrap();
    for dx in [-3.0, 0.0, 3.0] {
        let (got, want) = (at(&pm, e0 + dx), at(&plain, e0 + dx));
        assert!((got - want).abs() <= 0.02, "동작 흐림 90°: a vertical edge stays ({dx:+}: {got:.3} vs {want:.3})");
    }

    // unsharp: 64 | 192 grey step, radius 8 (sigma 4), amount 1
    let step_doc = || {
        let mut doc = Document::default();
        let (dk, lt) = (Color32::from_gray(64), Color32::from_gray(192));
        doc.insert("dark".into(), LayerKind::Shape(shape(ShapeKind::Rect, pos2(-200.0, -200.0), pos2(300.0, 300.0), dk, Some(dk), 0.0)), None);
        doc.insert("light".into(), LayerKind::Shape(shape(ShapeKind::Rect, pos2(50.0, -200.0), pos2(300.0, 300.0), lt, Some(lt), 0.0)), None);
        doc
    };
    let plain = draw(&step_doc()).unwrap();
    let g = |pm: &Pixmap, x: u32| pm.pixel(x, 50).unwrap().red() as f32;
    let x1 = (1..99).find(|&x| g(&plain, x) > 128.0).unwrap();
    let e1 = (x1 - 1) as f32 + 0.5 + (128.0 - g(&plain, x1 - 1)) / (g(&plain, x1) - g(&plain, x1 - 1));
    let mut doc = step_doc();
    doc.insert("fx".into(), LayerKind::Fx(FxLayer { v: [8.0, 1.0, 0.0, 0.0], ..FxLayer::new(Filter::Unsharp) }), None);
    let pm = draw(&doc).unwrap();
    let gv = |x: f32| {
        let i = (x - 0.5).floor();
        let f = x - 0.5 - i;
        g(&pm, i as u32) * (1.0 - f) + g(&pm, i as u32 + 1) * f
    };
    for (dx, want) in [(2.0, 231.5), (6.0, 200.5), (-2.0, 24.5), (-6.0, 55.5), (30.0, 192.0), (-30.0, 64.0)] {
        let got = gv(e1 + dx);
        assert!((got - want).abs() <= 6.0, "언샤프: at edge{dx:+} {got:.1}, want {want}");
    }
}

/// The look filters against their definitions (fx-layers.md, step 4), with
/// the export frame 0..100 as the frame. Skipped without a GPU.
#[test]
fn fx_look_filters() {
    use crate::doc::{Filter, FxLayer};
    let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0));
    let draw_at = |doc: &Document, c: egui::Pos2| crate::gpu::with_gpu(|g| g.render_ss(doc, c, region.size(), 0.0, 1.0, None, 1)).and_then(|r| r.ok());
    let rect = |doc: &mut Document, a: egui::Pos2, b: egui::Pos2, c: Color32| {
        doc.insert("r".into(), LayerKind::Shape(shape(ShapeKind::Rect, a, b, c, Some(c), 0.0)), None);
    };
    let with = |mut doc: Document, f: Filter, v: [f32; 4]| {
        doc.export_frame = Some(region);
        doc.insert("fx".into(), LayerKind::Fx(FxLayer { v, ..FxLayer::new(f) }), None);
        doc
    };
    let red = |pm: &Pixmap, x: u32, y: u32| pm.pixel(x, y).unwrap().red() as f32;
    let half = |pm: &Pixmap, ch: usize, y: u32| {
        // where a channel crosses half going right, sub-pixel
        let v = |x: u32| {
            let p = pm.pixel(x, y).unwrap();
            [p.red(), p.green(), p.blue(), p.alpha()][ch] as f32 / 255.0
        };
        let x0 = (1..99).find(|&x| (v(x) - 0.5) * (v(x - 1) - 0.5) <= 0.0).unwrap();
        (x0 - 1) as f32 + 0.5 + (v(x0 - 1) - 0.5) / (v(x0 - 1) - v(x0))
    };

    // vignette: flat grey 200; 양 .5, 크기 .5, 부드러움 .5 about the frame
    let mut grey = Document::default();
    rect(&mut grey, pos2(-50.0, -50.0), pos2(150.0, 150.0), Color32::from_gray(200));
    // no GPU: skipped (any other failure to draw fails below)
    if crate::gpu::with_gpu(|_| ()).is_none() {
        return;
    }
    let pm = draw_at(&with(grey.clone(), Filter::Vignette, [0.5, 0.5, 0.5, 0.0]), region.center()).unwrap();
    for (x, want) in [(50u32, 200.0), (87, 150.0), (99, 101.0)] {
        // pixel centres: x 50 → e ≈ 0.01, 87 → e = 0.75, 99 → e = 0.99
        let got = red(&pm, x, 50);
        assert!((got - want).abs() <= 3.0, "비네트 at x {x}: {got}, want {want}");
    }

    // pixelate 10: red | blue; the cell holding the edge is their mix, one colour
    let mut rb = Document::default();
    rect(&mut rb, pos2(-50.0, -50.0), pos2(150.0, 150.0), Color32::BLUE);
    rect(&mut rb, pos2(-50.0, -50.0), pos2(55.0, 150.0), Color32::RED);
    let plain = draw_at(&rb, region.center()).unwrap();
    let e = half(&plain, 0, 50);
    let pm = draw_at(&with(rb.clone(), Filter::Pixelate, [10.0, 0.0, 0.0, 0.0]), region.center()).unwrap();
    let cell = (e / 10.0).floor() as u32 * 10;
    let frac = (e - cell as f32) / 10.0;
    let p0 = pm.pixel(cell + 1, 41).unwrap();
    for (x, y) in [(cell + 8, 48), (cell + 4, 44)] {
        assert_eq!(pm.pixel(x, y).unwrap(), p0, "픽셀화: one colour per cell");
    }
    assert!((p0.red() as f32 - 255.0 * frac).abs() <= 8.0, "픽셀화: red {} for {frac:.2} red", p0.red());

    // colour fringes 10 px at the frame's edge: about a white edge at x≈77.5
    // on black, red moves out by 0.2·(x−50), blue in by the same
    let mut wb = Document::default();
    rect(&mut wb, pos2(-50.0, -50.0), pos2(150.0, 150.0), Color32::BLACK);
    rect(&mut wb, pos2(-50.0, -50.0), pos2(75.0, 150.0), Color32::WHITE);
    let plain = draw_at(&wb, region.center()).unwrap();
    let e = half(&plain, 1, 50);
    let pm = draw_at(&with(wb.clone(), Filter::Aberration, [10.0, 0.0, 0.0, 0.0]), region.center()).unwrap();
    let (xr, xg, xb) = (half(&pm, 0, 50), half(&pm, 1, 50), half(&pm, 2, 50));
    let (wr, wbl) = ((e - 10.0) / 0.8, (e + 10.0) / 1.2);
    assert!((xg - e).abs() < 0.3 && (xr - wr).abs() < 0.6 && (xb - wbl).abs() < 0.6, "색수차: edge {e:.2}; red {xr:.2} (want {wr:.2}), green {xg:.2}, blue {xb:.2} (want {wbl:.2})");

    // glow: white half on black, 반경 20 (sigma 10), 강도 1: outside the edge the
    // tail of the normal distribution, 255·Φ(−d/10)
    let pm = draw_at(&with(wb.clone(), Filter::Glow, [20.0, 1.0, 0.6, 0.0]), region.center()).unwrap();
    for (d, want) in [(5.0, 78.7), (10.0, 40.5), (20.0, 5.8)] {
        let x = e + d;
        let i = (x - 0.5).floor() as u32;
        let f = x - 0.5 - i as f32;
        let got = red(&pm, i, 50) * (1.0 - f) + red(&pm, i + 1, 50) * f;
        assert!((got - want).abs() <= 6.0, "글로우 {d} px out: {got:.1}, want {want}");
    }
    assert_eq!(red(&pm, 30, 50), 255.0);

    // halftone 8 px on 50 % grey: the dots' area is the ink, so the mean stays
    let mut mid = Document::default();
    rect(&mut mid, pos2(-50.0, -50.0), pos2(150.0, 150.0), Color32::from_gray(128));
    let pm = draw_at(&with(mid.clone(), Filter::Halftone, [8.0, 45.0, 0.0, 0.0]), region.center()).unwrap();
    let mut sum = 0.0;
    for y in 10..90 {
        for x in 10..90 {
            sum += red(&pm, x, y);
        }
    }
    let mean = sum / 6400.0 / 255.0;
    assert!((mean - 128.0 / 255.0).abs() <= 0.04, "하프톤: mean {mean:.3} for grey 0.502");

    // grain: fixed to the canvas, the same where the frame moves; the mean stays
    let gd = with(grey.clone(), Filter::Grain, [0.2, 1.0, 0.0, 0.0]);
    let a = draw_at(&gd, region.center()).unwrap();
    let b = draw_at(&gd, region.center() + vec2(20.0, 10.0)).unwrap();
    for (x, y) in [(40u32, 40u32), (60, 70), (75, 35)] {
        assert_eq!(a.pixel(x, y).unwrap(), b.pixel(x - 20, y - 10).unwrap(), "노이즈 at canvas {x},{y}");
    }
    let mut s = 0.0;
    let mut s2 = 0.0;
    for y in 0..100 {
        for x in 0..100 {
            let v = red(&a, x, y);
            s += v;
            s2 += v * v;
        }
    }
    let (m, sd) = (s / 1e4, (s2 / 1e4 - (s / 1e4).powi(2)).sqrt());
    assert!((m - 200.0).abs() < 4.0 && sd > 10.0, "노이즈: mean {m:.1}, sd {sd:.1}");

    // moving grain: the same within a frame, another pattern the next frame;
    // still grain the same in both
    let at = |moving: bool, t: f64| {
        let mut d = with(grey.clone(), Filter::Grain, [0.2, 1.0, 0.0, 0.0]);
        d.timeline.time = t;
        if let Some(LayerKind::Fx(f)) = d.layers.last_mut().map(|l| &mut l.kind) {
            f.moving = moving;
        }
        draw_at(&d, region.center()).unwrap()
    };
    let px = |pm: &Pixmap| (0..100).map(|i| pm.pixel(i, 37).unwrap().red()).collect::<Vec<_>>();
    let f0 = px(&at(true, 0.0));
    assert_eq!(f0, px(&at(true, 0.5 / 30.0)), "노이즈 (움직임): one pattern within a frame");
    let f1 = px(&at(true, 1.0 / 30.0));
    let same = f0.iter().zip(&f1).filter(|(a, b)| a == b).count();
    assert!(same < 20, "노이즈 (움직임): {same} of 100 pixels kept the next frame");
    assert_eq!(px(&at(false, 0.0)), px(&at(false, 1.0 / 30.0)), "노이즈: still without 프레임마다");

    // vignette about a filmed camera, 100 × 40 turned 90°: along canvas x its
    // short half (20) counts, so pixel 65 (15.5 right of the centre) is
    // e = 0.775; about the export frame (half 50) the same pixel is e = 0.31
    let vig = |e: f32| {
        let t = ((e - 0.5) / 0.5).clamp(0.0, 1.0);
        200.0 * (1.0 - 0.5 * t * t * (3.0 - 2.0 * t))
    };
    let mut cd = with(grey.clone(), Filter::Vignette, [0.5, 0.5, 0.5, 0.0]);
    cd.cameras.push(crate::anim::CanvasCam { id: 7, name: "cam".into(), rect: Rect::from_center_size(region.center(), vec2(100.0, 40.0)), anim: Default::default(), rot: std::f32::consts::FRAC_PI_2, follow: None });
    let frame = red(&draw_at(&cd, region.center()).unwrap(), 65, 50);
    cd.film = Some(crate::doc::Film::Camera(7));
    let pm = draw_at(&cd, region.center()).unwrap();
    let (cam, up) = (red(&pm, 65, 50), red(&pm, 50, 65));
    assert!((frame - 200.0).abs() <= 3.0 && (cam - vig(15.5 / 20.0)).abs() <= 3.0, "비네트: about the frame {frame}, about the camera {cam} (want {})", vig(15.5 / 20.0));
    // along canvas y the long half (50): e = 0.3
    assert!((up - 200.0).abs() <= 3.0, "비네트 (camera, y): {up}");
    // a fixed filmed region 30 wide: e = 15.5 / 15, the vignette's full 양
    cd.film = Some(crate::doc::Film::Fixed(Rect::from_center_size(region.center(), vec2(30.0, 100.0))));
    let fixed = red(&draw_at(&cd, region.center()).unwrap(), 65, 50);
    assert!((fixed - 100.0).abs() <= 3.0, "비네트 (fixed 30 wide): {fixed}");
}
