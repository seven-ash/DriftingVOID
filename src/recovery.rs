// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.

//! Recovery saves (notes/archive/2026-11-11-autosave.md): while a document has unsaved
//! changes, a copy of it is written now and then to
//! `%APPDATA%/DriftingVOID/recovery/<session>.void`, with a small
//! `<session>.json` beside it (the project's own path, if it has one). The
//! copy goes when the changes are saved or given up, so one that is still
//! there at the next start is of a session that ended without either (a
//! crash, a power cut, the task killed), and the app offers it.
//!
//! Each running app holds `<session>.lock` open without sharing: a copy whose
//! lock cannot be opened belongs to an app that is still running (a second
//! window) and is left alone.

use crate::doc::Document;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The recovery folder (`DV_RECOVERY_DIR` for tests and captures).
pub fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("DV_RECOVERY_DIR") {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("DriftingVOID").join("recovery"))
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Meta {
    /// The project file the document was opened from or saved to.
    #[serde(default)]
    path: Option<PathBuf>,
}

/// A recovery copy left by a session that is gone.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub file: PathBuf,
    /// The project it belongs to (None: never saved).
    pub origin: Option<PathBuf>,
    /// Seconds since it was written.
    pub age: u64,
}

impl Found {
    /// The project's file name, or None for a document never saved.
    pub fn name(&self) -> Option<String> {
        self.origin.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string())
    }

    fn side(&self, ext: &str) -> PathBuf {
        self.file.with_extension(ext)
    }

    /// Removes the copy (it was recovered into a session of its own, or the
    /// user gave it up).
    pub fn remove(&self) {
        for ext in ["void", "json", "lock", "void.tmp"] {
            let _ = std::fs::remove_file(self.side(ext));
        }
    }
}

/// Opens `p` for writing without sharing it (Windows); None when another
/// process holds it that way.
fn lock(p: &Path) -> Option<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.share_mode(0);
    }
    o.open(p).ok()
}

/// The copies in `dir` whose session is gone, newest first.
pub fn found_in(dir: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let file = e.path();
        // a lock nobody holds, with no copy beside it: a session that closed
        // with nothing unsaved (the window closed) left it
        if file.extension().and_then(|x| x.to_str()) == Some("lock") && !file.with_extension("void").exists() && lock(&file).is_some() {
            let _ = std::fs::remove_file(&file);
            continue;
        }
        if file.extension().and_then(|x| x.to_str()) != Some("void") {
            continue;
        }
        // its session still runs
        let lk = file.with_extension("lock");
        if lk.exists() && lock(&lk).is_none() {
            continue;
        }
        let meta: Meta = std::fs::read(file.with_extension("json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let age = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map_or(0, |d| d.as_secs());
        out.push(Found { file, origin: meta.path, age });
    }
    out.sort_by_key(|f| f.age);
    out
}

/// This run's recovery copy.
pub struct Session {
    base: PathBuf,
    _lock: Option<std::fs::File>,
    busy: Arc<AtomicBool>,
    /// Raised by `clear`: a save still running removes what it wrote.
    epoch: Arc<AtomicU64>,
    /// A copy is (or is being) on disk.
    has: bool,
}

impl Session {
    /// A session in `dir` (made if missing); None when it cannot be used.
    pub fn start(dir: &Path) -> Option<Self> {
        std::fs::create_dir_all(dir).ok()?;
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let base = dir.join(format!("{}-{t}", std::process::id()));
        let lk = lock(&base.with_extension("lock"));
        Some(Self { base, _lock: lk, busy: Default::default(), epoch: Default::default(), has: false })
    }

    fn file(&self, ext: &str) -> PathBuf {
        self.base.with_extension(ext)
    }

    /// A save is still being written.
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Relaxed)
    }

    /// Writes `doc` as this session's copy, on a thread. False when one is
    /// still being written (try again later).
    pub fn save(&mut self, doc: Document, origin: Option<PathBuf>) -> bool {
        if self.busy.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.has = true;
        let (void, json) = (self.file("void"), self.file("json"));
        let (busy, epoch) = (self.busy.clone(), self.epoch.clone());
        let at = epoch.load(Ordering::Acquire);
        std::thread::spawn(move || {
            let ok = crate::io::save_project(&doc, &void).is_ok();
            if ok {
                let _ = std::fs::write(&json, serde_json::to_vec(&Meta { path: origin }).unwrap_or_default());
            }
            // cleared meanwhile (saved, given up): nothing is left behind
            if epoch.load(Ordering::Acquire) != at {
                let _ = std::fs::remove_file(&void);
                let _ = std::fs::remove_file(&json);
            }
            busy.store(false, Ordering::Release);
        });
        true
    }

    /// The changes are saved or given up: the copy goes.
    pub fn clear(&mut self) {
        if !self.has {
            return;
        }
        self.has = false;
        self.epoch.fetch_add(1, Ordering::AcqRel);
        for ext in ["void", "json", "void.tmp"] {
            let _ = std::fs::remove_file(self.file(ext));
        }
    }

    /// Takes a found copy as this session's own (it was just opened): it
    /// stays on disk under this session until the first save here replaces
    /// it, so a second crash right away loses nothing.
    pub fn adopt(&mut self, f: &Found) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        let moved = std::fs::rename(&f.file, self.file("void")).is_ok();
        let _ = std::fs::rename(f.side("json"), self.file("json"));
        let _ = std::fs::remove_file(f.side("lock"));
        self.has = moved;
    }

    /// The app is closing in order: the copy and the lock go.
    pub fn end(&mut self) {
        self.clear();
        self._lock = None;
        let _ = std::fs::remove_file(self.file("lock"));
    }
}

/// How long a note left by `leave_reopen` counts: an update takes a minute
/// or two; an older note is of an update that never started the app again.
const REOPEN_FOR: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Before an update's installer runs: notes the open project in `dir`, so
/// the app started after the update opens it again (Ash, 11-12).
pub fn leave_reopen(dir: &Path, project: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join("reopen.txt"), project.to_string_lossy().as_bytes());
}

/// At start: the project noted by `leave_reopen`, once (the note goes), if it
/// is recent and the project is still there.
pub fn take_reopen(dir: &Path) -> Option<PathBuf> {
    let note = dir.join("reopen.txt");
    let fresh = std::fs::metadata(&note).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d < REOPEN_FOR);
    let text = std::fs::read_to_string(&note).ok();
    let _ = std::fs::remove_file(&note);
    text.filter(|_| fresh).map(|t| PathBuf::from(t.trim())).filter(|p| p.is_file())
}

/// "3분 전", "2시간 전", "5일 전" for an age in seconds.
pub fn ago(secs: u64) -> String {
    match secs {
        0..=59 => crate::i18n::tr("방금").to_string(),
        60..=3599 => crate::i18n::trf("{}분 전", &[format!("{}", secs / 60)]),
        3600..=86_399 => crate::i18n::trf("{}시간 전", &[format!("{}", secs / 3600)]),
        _ => crate::i18n::trf("{}일 전", &[format!("{}", secs / 86_400)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{LayerKind, ShapeKind, ShapeLayer};
    use egui::{Color32, pos2};

    fn doc() -> Document {
        let mut d = Document::default();
        d.insert("r".into(), LayerKind::Shape(ShapeLayer { kind: ShapeKind::Rect, a: pos2(0.0, 0.0), b: pos2(10.0, 10.0), color: Color32::RED, width: 2.0, fill: None, rotation: 0.0 }), None);
        d
    }

    fn wait(s: &Session) {
        for _ in 0..400 {
            if !s.busy() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the save did not finish");
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dv_recovery_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// The project open before an update is opened once after it.
    #[test]
    fn the_project_is_noted_across_an_update() {
        let dir = temp("reopen");
        assert_eq!(take_reopen(&dir), None);
        std::fs::create_dir_all(dir.join("work")).unwrap();
        let project = dir.join("work").join("포스터 1.void");
        std::fs::write(&project, b"x").unwrap();
        leave_reopen(&dir, &project);
        assert!(found_in(&dir).is_empty(), "the note is not a recovery copy");
        assert_eq!(take_reopen(&dir), Some(project.clone()));
        assert_eq!(take_reopen(&dir), None, "once");
        // a project that is gone is not opened
        leave_reopen(&dir, &dir.join("gone.void"));
        assert_eq!(take_reopen(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A copy shows up for the next start only once its session is gone; it
    /// opens as the document, and clearing or ending leaves nothing behind.
    #[test]
    fn a_copy_is_found_after_its_session_is_gone() {
        let dir = temp("found");
        let mut s = Session::start(&dir).unwrap();
        assert!(s.save(doc(), Some(PathBuf::from("C:/work/poster.void"))));
        wait(&s);
        // the session still runs (its lock is held): not offered
        #[cfg(windows)]
        assert!(found_in(&dir).is_empty());
        // the session dies without clearing (a crash): its lock is released
        let file = s.file("void");
        s._lock = None;
        s.has = false;
        drop(s);
        let f = found_in(&dir);
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].file.clone(), f[0].name().as_deref()), (file, Some("poster.void")));
        assert_eq!(crate::io::load_project(&f[0].file).unwrap().layers.len(), 1);

        // recovered into a new session: it is that session's copy now
        let mut s2 = Session::start(&dir).unwrap();
        s2.adopt(&f[0]);
        assert!(s2.file("void").exists() && !f[0].file.exists());
        #[cfg(windows)]
        assert!(found_in(&dir).is_empty());
        // saved: nothing is left; ending removes the lock too
        s2.clear();
        assert!(!s2.file("void").exists() && !s2.file("json").exists());
        s2.end();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Clearing while a save is being written leaves no copy either.
    #[test]
    fn clearing_during_a_save_leaves_nothing() {
        let dir = temp("race");
        let mut s = Session::start(&dir).unwrap();
        assert!(s.save(doc(), None));
        s.clear();
        wait(&s);
        assert!(!s.file("void").exists() && !s.file("json").exists());
        // and a copy given up is removed whole
        assert!(s.save(doc(), None));
        wait(&s);
        s._lock = None;
        let f = Found { file: s.file("void"), origin: None, age: 0 };
        f.remove();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ages_read_in_minutes_hours_days() {
        // (whatever the language: the numbers)
        assert!(!ago(30).contains(|c: char| c.is_ascii_digit()));
        assert!(ago(200).contains('3'));
        assert!(ago(7300).contains('2'));
        assert!(ago(86_400 * 5 + 10).contains('5'));
    }
}
