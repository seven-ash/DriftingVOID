// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.

//! Updates from the GitHub releases (Ash, 11-02): no pop-up. A release build
//! asks GitHub once at start (an off switch in 설정) for the latest release;
//! a newer one shows as a quiet notice right of the top bar's menus
//! ("current → new") and as a row in 설정. A click downloads its installer,
//! checks the SHA-256 GitHub publishes for it, and quits through the usual
//! unsaved-changes question; the installer then runs silently and starts the
//! app again (`/relaunch=1`, `dist/inno/DriftingVOID.iss`).
//!
//! The network goes through Windows' own curl.exe (Windows 10 1803+), so the
//! app carries no HTTP or TLS stack. Dev builds (no release version) never
//! check. Neither does a build run from an unzipped folder (Ash, 11-12): the
//! installer would put the new version elsewhere and leave that folder as it
//! was, so the unzipped build stays the version it is.
//!
//! Captures and tests: `DV_UPDATE_CURRENT=<version>` stands in for the
//! release version (a dev build then checks too), `DV_UPDATE_FAKE=<version>`
//! shows that release as available without asking GitHub,
//! `DV_UPDATE_NOLAUNCH=1` stops after the download is checked,
//! `DV_UPDATE_AUTO=1` downloads an available release without a click.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const API: &str = "https://api.github.com/repos/seven-ash/DriftingVOID/releases/latest";

/// A release newer than this build.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Not asked (dev build, switched off, or not yet).
    Idle,
    Checking,
    /// This build is the latest release.
    Latest,
    Available(Release),
    /// Downloading: done fraction 0..1.
    Downloading(Release, f32),
    /// Downloaded and checked: the installer to run.
    Ready(Release, PathBuf),
    /// What went wrong (shown as a tip); a click tries again.
    Failed(String),
}

#[derive(Clone)]
pub struct Updater {
    state: Arc<Mutex<State>>,
    ctx: Option<egui::Context>,
}

/// The release this build is ("" for a dev build).
pub fn current() -> String {
    std::env::var("DV_UPDATE_CURRENT").unwrap_or_else(|_| env!("DV_RELEASE").to_string())
}

/// Was this build put here by the installer? (Its uninstaller lies beside
/// the program.) An unzipped build has no updates. `DV_UPDATE_CURRENT` and
/// `DV_UPDATE_FAKE` count as installed, for captures and tests.
pub fn installed() -> bool {
    static I: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *I.get_or_init(|| {
        if std::env::var_os("DV_UPDATE_CURRENT").is_some() || std::env::var_os("DV_UPDATE_FAKE").is_some() {
            return true;
        }
        let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
        dir.and_then(|d| std::fs::read_dir(d).ok()).is_some_and(|rd| {
            rd.flatten().any(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                n.starts_with("unins") && n.ends_with(".exe")
            })
        })
    })
}

/// Is release version `a` newer than `b`? Their numbers compared in order
/// (1.0.10.R > 1.0.9.R); a suffix such as ".R" does not count.
pub fn newer(a: &str, b: &str) -> bool {
    let nums = |s: &str| -> Vec<u64> { s.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).filter_map(|x| x.parse().ok()).collect() };
    let (x, y) = (nums(a), nums(b));
    for i in 0..x.len().max(y.len()) {
        let (p, q) = (x.get(i).copied().unwrap_or(0), y.get(i).copied().unwrap_or(0));
        if p != q {
            return p > q;
        }
    }
    false
}

fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn curl() -> Command {
    // Windows' own, by its full path: a bare name is looked for in the app's
    // folder first, which the user can write to
    let sys = std::env::var_os("SystemRoot").map(|r| std::path::Path::new(&r).join("System32").join("curl.exe")).filter(|p| p.is_file());
    let mut c = match sys {
        Some(p) => Command::new(p),
        None => Command::new("curl.exe"),
    };
    no_window(&mut c);
    c.args(["-sSfL", "--max-time", "600", "-H", &format!("User-Agent: DriftingVOID/{}", current())]);
    c
}

/// The latest release from the API's JSON: its version (the tag without a
/// `release-` or `v` in front), its installer and that file's checksum.
pub fn parse_latest(json: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("no tag")?;
    let version = tag.trim_start_matches("release-").trim_start_matches('v').to_string();
    let asset = v["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str().is_some_and(|n| n.ends_with("-Setup.exe")))).ok_or("no installer in the release")?;
    let sha256 = asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).filter(|d| d.len() == 64).ok_or("no checksum for the installer")?.to_lowercase();
    Ok(Release {
        version,
        url: asset["browser_download_url"].as_str().ok_or("no download link")?.to_string(),
        name: asset["name"].as_str().unwrap_or("DriftingVOID-Setup.exe").to_string(),
        size: asset["size"].as_u64().unwrap_or(0),
        sha256,
    })
}

fn sha256_file(p: &std::path::Path) -> std::io::Result<String> {
    use sha2::Digest;
    let mut f = std::fs::File::open(p)?;
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

impl Updater {
    pub fn new() -> Self {
        Updater { state: Arc::new(Mutex::new(State::Idle)), ctx: None }
    }

    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or(State::Idle)
    }

    fn set(&self, s: State) {
        if let Ok(mut g) = self.state.lock() {
            *g = s;
        }
        if let Some(c) = &self.ctx {
            c.request_repaint();
        }
    }

    /// Asks GitHub in the background (a release build, or DV_UPDATE_*).
    pub fn check(&mut self, ctx: &egui::Context) {
        self.ctx = Some(ctx.clone());
        if let Ok(v) = std::env::var("DV_UPDATE_FAKE") {
            self.set(State::Available(Release { version: v, url: String::new(), name: String::new(), size: 0, sha256: String::new() }));
            return;
        }
        let cur = current();
        // (a downloaded installer stays ready: asking again would drop it)
        if cur.is_empty() || !installed() || matches!(self.state(), State::Checking | State::Downloading(..) | State::Ready(..)) {
            return;
        }
        self.set(State::Checking);
        let me = self.clone();
        std::thread::spawn(move || {
            let out = curl().args(["-H", "Accept: application/vnd.github+json", API]).stderr(Stdio::null()).output();
            let s = match out {
                Ok(o) if o.status.success() => match parse_latest(&String::from_utf8_lossy(&o.stdout)) {
                    Ok(r) if newer(&r.version, &cur) => State::Available(r),
                    Ok(_) => {
                        // up to date: an installer downloaded for this version has done its work
                        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("DriftingVOID-update"));
                        State::Latest
                    }
                    Err(e) => State::Failed(e),
                },
                Ok(o) => State::Failed(format!("GitHub: curl {}", o.status.code().unwrap_or(-1))),
                Err(e) => State::Failed(format!("curl: {e}")),
            };
            me.set(s);
        });
    }

    /// Downloads the installer of an available release and checks it.
    pub fn download(&mut self, ctx: &egui::Context) {
        self.ctx = Some(ctx.clone());
        let State::Available(r) = self.state() else { return };
        if r.url.is_empty() {
            return; // DV_UPDATE_FAKE
        }
        self.set(State::Downloading(r.clone(), 0.0));
        let me = self.clone();
        std::thread::spawn(move || {
            let dir = std::env::temp_dir().join("DriftingVOID-update");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(&r.name);
            let _ = std::fs::remove_file(&path);
            let child = curl().args(["-o"]).arg(&path).arg(&r.url).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(e) => return me.set(State::Failed(format!("curl: {e}"))),
            };
            // progress: the file's size against the release's
            let ok = loop {
                match child.try_wait() {
                    Ok(Some(st)) => break st.success(),
                    Ok(None) => {
                        let got = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        if r.size > 0 {
                            me.set(State::Downloading(r.clone(), (got as f32 / r.size as f32).min(1.0)));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                    Err(_) => break false,
                }
            };
            if !ok {
                return me.set(State::Failed("download failed".into()));
            }
            match sha256_file(&path) {
                Ok(h) if h == r.sha256 => me.set(State::Ready(r, path)),
                Ok(h) => {
                    let _ = std::fs::remove_file(&path);
                    me.set(State::Failed(format!("checksum mismatch ({h})")))
                }
                Err(e) => me.set(State::Failed(e.to_string())),
            }
        });
    }

    /// Checks the downloaded installer once more (it has sat in the temp
    /// folder since the download) and starts it; false, with the state set
    /// to what went wrong, when it is gone, changed or would not start. The
    /// caller quits only on true.
    pub fn install(&self, path: &std::path::Path) -> bool {
        let want = match self.state() {
            State::Ready(r, p) if p == path => r.sha256,
            _ => String::new(),
        };
        // a pretend release (DV_UPDATE_FAKE) has no checksum
        if !want.is_empty() {
            match sha256_file(path) {
                Ok(h) if h == want => {}
                Ok(_) => {
                    let _ = std::fs::remove_file(path);
                    self.set(State::Failed("the installer changed after the download".into()));
                    return false;
                }
                Err(e) => {
                    self.set(State::Failed(format!("the installer is gone ({e})")));
                    return false;
                }
            }
        }
        if !Self::launch(path) {
            self.set(State::Failed("the installer did not start".into()));
            return false;
        }
        true
    }

    /// Starts the checked installer, silent, to start the app again when
    /// done; the caller quits right after.
    pub fn launch(path: &std::path::Path) -> bool {
        if std::env::var_os("DV_UPDATE_NOLAUNCH").is_some() {
            return true;
        }
        Command::new(path).args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/relaunch=1"]).spawn().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_versions_compare_by_their_numbers() {
        assert!(newer("1.0.1.R", "1.0.0.R"));
        assert!(newer("1.0.10.R", "1.0.9.R"));
        assert!(newer("1.1", "1.0.9.R"));
        assert!(!newer("1.0.1.R", "1.0.1.R"));
        assert!(!newer("1.0.0.R", "1.0.1.R"));
    }

    #[test]
    fn latest_release_from_the_api() {
        let json = r#"{"tag_name":"release-1.0.2.R","assets":[
            {"name":"DriftingVOID-1.0.2.R.zip","browser_download_url":"https://x/z","size":5,"digest":"sha256:00"},
            {"name":"DriftingVOID-1.0.2.R-Setup.exe","browser_download_url":"https://x/s","size":1234,
             "digest":"sha256:ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789"}]}"#;
        let r = parse_latest(json).unwrap();
        assert_eq!((r.version.as_str(), r.url.as_str(), r.size), ("1.0.2.R", "https://x/s", 1234));
        assert_eq!(r.sha256, "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789");
        // no checksum: refused
        assert!(parse_latest(r#"{"tag_name":"v2","assets":[{"name":"a-Setup.exe","browser_download_url":"u","size":1}]}"#).is_err());
    }
}
