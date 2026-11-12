// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (AGPL-3.0 section 7): see NOTICE.

//! Installed system fonts (Windows Fonts folder + per-user fonts), scanned in
//! the background, loaded on demand.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use ab_glyph::FontArc;

#[derive(Clone)]
pub struct Face {
    pub style: String,
    pub path: PathBuf,
    pub index: u32,
    pub weight: u16,
    pub italic: bool,
}

pub struct Family {
    pub name: String,
    pub faces: Vec<Face>,
}

impl Family {
    /// The face closest to a regular upright weight.
    pub fn regular(&self) -> &Face {
        self.faces
            .iter()
            .min_by_key(|f| (f.italic as i32 * 1000) + (f.weight as i32 - 400).abs())
            .unwrap()
    }
}

#[derive(Clone)]
pub struct Loaded {
    pub font: FontArc,
    pub bytes: Arc<Vec<u8>>,
    pub index: u32,
}

static FAMILIES: OnceLock<Arc<Vec<Family>>> = OnceLock::new();
static LOADED: OnceLock<Mutex<HashMap<(String, String), Option<Loaded>>>> = OnceLock::new();

pub fn start_scan() {
    std::thread::spawn(|| {
        let _ = FAMILIES.set(Arc::new(scan()));
    });
}

/// None while the background scan is still running.
pub fn families() -> Option<Arc<Vec<Family>>> {
    FAMILIES.get().cloned()
}

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let win = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    dirs.push(Path::new(&win).join("Fonts"));
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        dirs.push(Path::new(&local).join("Microsoft\\Windows\\Fonts"));
    }
    // the faces shipped with the app (`DEFAULT_FAMILY`): the `fonts` folder
    // beside the program, else the repository's `assets/fonts` (builds run
    // from the tree)
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("fonts"))).filter(|d| d.is_dir());
    dirs.push(beside.unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("fonts")));
    dirs
}

/// The family a new text starts in (Ash, 11-12): shipped with the app, with
/// Hangul, kana, kanji and Latin. (A text with no family named is of a file
/// from before and keeps the default it was made with, Malgun Gothic.)
pub const DEFAULT_FAMILY: &str = "Pretendard JP";

fn pick_name(face: &ttf_parser::Face, ids: &[u16]) -> Option<String> {
    // Prefer a Korean name, then English, then anything readable.
    for &id in ids {
        let names: Vec<_> = face.names().into_iter().filter(|n| n.name_id == id && n.is_unicode()).collect();
        for lang in [0x0412u16, 0x0409] {
            if let Some(s) = names.iter().filter(|n| n.language_id == lang).find_map(|n| n.to_string()) {
                return Some(s);
            }
        }
        if let Some(s) = names.iter().find_map(|n| n.to_string()) {
            return Some(s);
        }
    }
    None
}

fn english_name(face: &ttf_parser::Face, ids: &[u16]) -> Option<String> {
    for &id in ids {
        if let Some(s) = face
            .names()
            .into_iter()
            .filter(|n| n.name_id == id && n.is_unicode() && n.language_id == 0x0409)
            .find_map(|n| n.to_string())
        {
            return Some(s);
        }
    }
    pick_name(face, ids)
}

fn scan() -> Vec<Family> {
    let mut map: HashMap<String, Family> = HashMap::new();
    for dir in font_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
                continue;
            }
            let Ok(file) = std::fs::File::open(&path) else { continue };
            // SAFETY: font files are read-only system resources; a concurrent
            // truncation would at worst make this scan fail for that file.
            let Ok(map_data) = (unsafe { memmap2::Mmap::map(&file) }) else { continue };
            let n = ttf_parser::fonts_in_collection(&map_data).unwrap_or(1);
            for index in 0..n {
                let Ok(face) = ttf_parser::Face::parse(&map_data, index) else { continue };
                let Some(family) = pick_name(&face, &[16, 1]) else { continue };
                let style = english_name(&face, &[17, 2]).unwrap_or_else(|| "Regular".into());
                let fam = map.entry(family.clone()).or_insert_with(|| Family { name: family, faces: Vec::new() });
                if fam.faces.iter().any(|f| f.style == style) {
                    continue;
                }
                fam.faces.push(Face {
                    style,
                    path: path.clone(),
                    index,
                    weight: face.weight().to_number(),
                    italic: face.is_italic(),
                });
            }
        }
    }
    let mut v: Vec<Family> = map.into_values().filter(|f| !f.name.starts_with('@')).collect();
    for f in &mut v {
        f.faces.sort_by_key(|x| (x.italic, x.weight));
    }
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v
}

pub fn find(family: &str, style: &str) -> Option<Face> {
    let fams = families()?;
    let f = fams.iter().find(|f| f.name == family)?;
    Some(f.faces.iter().find(|x| x.style == style).unwrap_or_else(|| f.regular()).clone())
}

/// Loads (and caches) a face. Returns None if unknown or not yet scanned.
pub fn load(family: &str, style: &str) -> Option<Loaded> {
    let cache = LOADED.get_or_init(Default::default);
    let key = (family.to_string(), style.to_string());
    if let Some(v) = cache.lock().unwrap().get(&key) {
        return v.clone();
    }
    // Before the scan finishes we can't resolve names; don't cache the miss.
    families()?;
    let res = find(family, style).and_then(|face| {
        let bytes = std::fs::read(&face.path).ok()?;
        let font = FontArc::new(ab_glyph::FontVec::try_from_vec_and_index(bytes.clone(), face.index).ok()?);
        Some(Loaded { font, bytes: Arc::new(bytes), index: face.index })
    });
    cache.lock().unwrap().insert(key, res.clone());
    res
}
