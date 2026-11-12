// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (AGPL-3.0 section 7): see NOTICE.

//! Drops from other programs, browsers included. winit's drop target takes
//! only real files (CF_HDROP), so an image dragged out of Chrome or Edge (a
//! virtual file: FileGroupDescriptorW + FileContents, plus its URL) was
//! refused. This replaces it with our own OLE IDropTarget that takes, in
//! order: files, virtual files, PNG data, an image URL (data: decoded,
//! http(s) downloaded on a worker thread), then plain text. The app drains
//! `take()` each frame.

use std::path::PathBuf;
use std::sync::Mutex;

/// Something dropped on the window. `at` is the drop point in client pixels.
#[derive(Debug)]
pub enum Dropped {
    Paths(Vec<PathBuf>),
    /// A file's bytes without a file (browser image, data: URL, download);
    /// `url` when they came from a link (not an image: the link as text).
    Bytes { name: String, bytes: Vec<u8>, url: Option<String> },
    /// A link that couldn't be fetched.
    NoFetch { url: String, why: String },
    Text(String),
    Failed(String),
}

static QUEUE: Mutex<Vec<(Dropped, Option<[f32; 2]>)>> = Mutex::new(Vec::new());
static CTX: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

fn push(d: Dropped, at: Option<[f32; 2]>) {
    if let Ok(mut q) = QUEUE.lock() {
        q.push((d, at));
    }
    if let Some(c) = CTX.get() {
        c.request_repaint();
    }
}

/// Drops since the last call.
pub fn take() -> Vec<(Dropped, Option<[f32; 2]>)> {
    QUEUE.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// File name for downloaded or decoded bytes: the URL's last path part, or
/// `image` with the extension its bytes look like.
pub fn name_for(url: &str, bytes: &[u8]) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let last = path.rsplit('/').next().unwrap_or("");
    if !url.starts_with("data:") && last.contains('.') && last.len() <= 120 {
        return percent_decode(last);
    }
    let ext = match bytes {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xFF, 0xD8, ..] => "jpg",
        [b'G', b'I', b'F', ..] => "gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        [b'B', b'M', ..] => "bmp",
        _ => "png",
    };
    format!("image.{ext}")
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            // by bytes: the two after a % may be half of a multi-byte character
            // (a slice of the str there would panic)
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The bytes of a `data:` URL (base64 or percent-encoded).
pub fn data_url(url: &str) -> Option<Vec<u8>> {
    let rest = url.strip_prefix("data:")?;
    let (meta, body) = rest.split_once(',')?;
    if meta.ends_with(";base64") { base64(body) } else { Some(percent_decode(body).into_bytes()) }
}

fn base64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Is this text a link to fetch rather than words to place?
pub fn is_url(t: &str) -> bool {
    let t = t.trim();
    !t.contains(char::is_whitespace) && (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("data:image/"))
}

/// Turns a URL into bytes: decoded at once, or downloaded on a worker thread
/// (the result is queued like a drop).
pub fn fetch(url: String, at: Option<[f32; 2]>) {
    if url.starts_with("data:") {
        match data_url(&url) {
            Some(bytes) => push(Dropped::Bytes { name: name_for(&url, &bytes), bytes, url: Some(url) }, at),
            None => push(Dropped::NoFetch { url, why: "data:".into() }, at),
        }
        return;
    }
    std::thread::spawn(move || match download(&url) {
        Ok(bytes) => push(Dropped::Bytes { name: name_for(&url, &bytes), bytes, url: Some(url) }, at),
        Err(why) => push(Dropped::NoFetch { url, why }, at),
    });
}

#[cfg(windows)]
fn download(url: &str) -> Result<Vec<u8>, String> {
    let w: Vec<u16> = url.encode_utf16().chain([0]).collect();
    let mut file = [0u16; 1024];
    // SAFETY: plain Win32 calls with valid buffers; COM is initialised for
    // this worker thread first (urlmon needs it).
    let hr = unsafe {
        sys::CoInitializeEx(std::ptr::null(), 2);
        sys::URLDownloadToCacheFileW(std::ptr::null_mut(), w.as_ptr(), file.as_mut_ptr(), file.len() as u32, 0, std::ptr::null_mut())
    };
    if hr < 0 {
        return Err(format!("0x{:08X}", hr as u32));
    }
    // read into memory, then drop the cache copy: a photo is kept inside the
    // .void, so nothing on disk needs to stay
    let n = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    let path = String::from_utf16_lossy(&file[..n]);
    let bytes = std::fs::read(&path).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&path);
    bytes
}

#[cfg(not(windows))]
fn download(url: &str) -> Result<Vec<u8>, String> {
    Err(url.to_string())
}

#[cfg(not(windows))]
pub fn install(_: &dyn raw_window_handle::HasWindowHandle, _: &egui::Context) {}

#[cfg(windows)]
pub use win::{from_clipboard, install};

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    #[repr(C)]
    pub struct FormatEtc {
        pub cf: u16,
        pub ptd: *mut c_void,
        pub aspect: u32,
        pub lindex: i32,
        pub tymed: u32,
    }

    /// STGMEDIUM: its union is one pointer-sized handle.
    #[repr(C)]
    pub struct StgMedium {
        pub tymed: u32,
        pub h: *mut c_void,
        pub release: *mut c_void,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct PointL {
        pub x: i32,
        pub y: i32,
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        pub fn RegisterDragDrop(hwnd: isize, target: *mut c_void) -> i32;
        pub fn RevokeDragDrop(hwnd: isize) -> i32;
        pub fn ReleaseStgMedium(m: *mut StgMedium);
        pub fn OleGetClipboard(obj: *mut *mut c_void) -> i32;
        pub fn CoInitializeEx(reserved: *const c_void, coinit: u32) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn RegisterClipboardFormatW(name: *const u16) -> u32;
        pub fn ScreenToClient(hwnd: isize, p: *mut PointL) -> i32;
        pub fn GetCursorPos(p: *mut PointL) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GlobalLock(h: *mut c_void) -> *mut c_void;
        pub fn GlobalUnlock(h: *mut c_void) -> i32;
        pub fn GlobalSize(h: *mut c_void) -> usize;
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        pub fn DragQueryFileW(hdrop: *mut c_void, i: u32, buf: *mut u16, cch: u32) -> u32;
    }
    #[link(name = "urlmon")]
    unsafe extern "system" {
        pub fn URLDownloadToCacheFileW(caller: *mut c_void, url: *const u16, file: *mut u16, cch: u32, reserved: u32, cb: *mut c_void) -> i32;
    }
}

#[cfg(windows)]
mod win {
    use super::sys::*;
    use super::{Dropped, push};
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    const CF_UNICODETEXT: u16 = 13;
    const CF_HDROP: u16 = 15;
    const TYMED_HGLOBAL: u32 = 1;
    const TYMED_ISTREAM: u32 = 4;
    const DVASPECT_CONTENT: u32 = 1;
    const DROPEFFECT_NONE: u32 = 0;
    const DROPEFFECT_COPY: u32 = 1;
    const S_OK: i32 = 0;
    const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

    // ------------------------------------------------------------ the data object

    /// An IDataObject (borrowed): vtable slots 3 GetData, 5 QueryGetData.
    struct Data(*mut c_void);

    type GetData = unsafe extern "system" fn(*mut c_void, *const FormatEtc, *mut StgMedium) -> i32;
    type QueryGetData = unsafe extern "system" fn(*mut c_void, *const FormatEtc) -> i32;

    fn cf(name: &str) -> u16 {
        let w: Vec<u16> = name.encode_utf16().chain([0]).collect();
        // SAFETY: nul-terminated name
        unsafe { RegisterClipboardFormatW(w.as_ptr()) as u16 }
    }

    fn fmt(cf: u16, lindex: i32, tymed: u32) -> FormatEtc {
        FormatEtc { cf, ptd: std::ptr::null_mut(), aspect: DVASPECT_CONTENT, lindex, tymed }
    }

    impl Data {
        fn vt(&self, slot: usize) -> *const c_void {
            // SAFETY: a COM object starts with its vtable pointer
            unsafe { *(*(self.0 as *const *const *const c_void)).add(slot) }
        }

        fn has(&self, cf: u16) -> bool {
            let f = fmt(cf, -1, TYMED_HGLOBAL | TYMED_ISTREAM);
            // SAFETY: slot 5 is QueryGetData
            unsafe { std::mem::transmute::<*const c_void, QueryGetData>(self.vt(5))(self.0, &f) == S_OK }
        }

        /// The bytes of a format (HGLOBAL or IStream).
        fn get(&self, cf: u16, lindex: i32) -> Option<Vec<u8>> {
            let f = fmt(cf, lindex, TYMED_HGLOBAL | TYMED_ISTREAM);
            let mut m = StgMedium { tymed: 0, h: std::ptr::null_mut(), release: std::ptr::null_mut() };
            // SAFETY: slot 3 is GetData; the medium is released below
            let hr = unsafe { std::mem::transmute::<*const c_void, GetData>(self.vt(3))(self.0, &f, &mut m) };
            if hr != S_OK {
                return None;
            }
            let out = match m.tymed {
                TYMED_HGLOBAL => global_bytes(m.h),
                TYMED_ISTREAM => stream_bytes(m.h),
                _ => None,
            };
            // SAFETY: medium filled by GetData
            unsafe { ReleaseStgMedium(&mut m) };
            out
        }

        fn paths(&self) -> Option<Vec<PathBuf>> {
            let f = fmt(CF_HDROP, -1, TYMED_HGLOBAL);
            let mut m = StgMedium { tymed: 0, h: std::ptr::null_mut(), release: std::ptr::null_mut() };
            // SAFETY: as in `get`; the HDROP is read with DragQueryFileW
            unsafe {
                if std::mem::transmute::<*const c_void, GetData>(self.vt(3))(self.0, &f, &mut m) != S_OK {
                    return None;
                }
                let n = DragQueryFileW(m.h, u32::MAX, std::ptr::null_mut(), 0);
                let mut out = Vec::new();
                for i in 0..n {
                    let len = DragQueryFileW(m.h, i, std::ptr::null_mut(), 0) as usize;
                    let mut buf = vec![0u16; len + 1];
                    DragQueryFileW(m.h, i, buf.as_mut_ptr(), buf.len() as u32);
                    out.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
                }
                ReleaseStgMedium(&mut m);
                Some(out)
            }
        }

        fn text(&self, cf: u16) -> Option<String> {
            let b = self.get(cf, -1)?;
            let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            let s = String::from_utf16_lossy(&w);
            (!s.trim().is_empty()).then_some(s)
        }

        /// Virtual files: (name, bytes) for each non-folder entry.
        fn virtual_files(&self) -> Vec<(String, Vec<u8>)> {
            let Some(desc) = self.get(cf("FileGroupDescriptorW"), -1) else { return Vec::new() };
            if desc.len() < 4 {
                return Vec::new();
            }
            let n = u32::from_le_bytes([desc[0], desc[1], desc[2], desc[3]]) as usize;
            // FILEDESCRIPTORW: 592 bytes, packed; flags at 0, attributes at 36, name at 72
            const SIZE: usize = 592;
            let contents = cf("FileContents");
            let mut out = Vec::new();
            for i in 0..n {
                let base = 4 + i * SIZE;
                let Some(d) = desc.get(base..base + SIZE) else { break };
                let attrs = u32::from_le_bytes([d[36], d[37], d[38], d[39]]);
                let flags = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
                // FD_ATTRIBUTES (0x4) says whether `attrs` is filled in
                if flags & 0x4 != 0 && attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    continue;
                }
                let w: Vec<u16> = d[72..72 + 520].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
                let name = String::from_utf16_lossy(&w);
                // by index; some sources only answer lindex −1 (one file)
                let got = self.get(contents, i as i32).or_else(|| if n == 1 { self.get(contents, -1) } else { None });
                if let Some(bytes) = got.filter(|b| !b.is_empty()) {
                    out.push((name, bytes));
                }
            }
            out
        }

        fn usable(&self) -> bool {
            [CF_HDROP, cf("FileGroupDescriptorW"), cf("PNG"), cf("UniformResourceLocatorW"), CF_UNICODETEXT].into_iter().any(|c| self.has(c))
        }

        /// What this data object carries, in order of preference.
        fn take(&self, at: Option<[f32; 2]>) {
            if let Some(p) = self.paths().filter(|p| !p.is_empty()) {
                return push(Dropped::Paths(p), at);
            }
            let files = self.virtual_files();
            if !files.is_empty() {
                for (name, bytes) in files {
                    push(Dropped::Bytes { name, bytes, url: None }, at);
                }
                return;
            }
            if let Some(bytes) = self.get(cf("PNG"), -1) {
                return push(Dropped::Bytes { name: "image.png".into(), bytes, url: None }, at);
            }
            let url = self.text(cf("UniformResourceLocatorW")).or_else(|| self.text(CF_UNICODETEXT).filter(|t| super::is_url(t)));
            if let Some(u) = url {
                return super::fetch(u.trim().to_string(), at);
            }
            if let Some(t) = self.text(CF_UNICODETEXT) {
                push(Dropped::Text(t), at);
            }
        }
    }

    fn global_bytes(h: *mut c_void) -> Option<Vec<u8>> {
        // SAFETY: an HGLOBAL from GetData, locked while copied
        unsafe {
            let p = GlobalLock(h) as *const u8;
            if p.is_null() {
                return None;
            }
            let v = std::slice::from_raw_parts(p, GlobalSize(h)).to_vec();
            GlobalUnlock(h);
            Some(v)
        }
    }

    fn stream_bytes(s: *mut c_void) -> Option<Vec<u8>> {
        // IStream slot 3: Read(pv, cb, pcbRead); S_FALSE (1) at the end
        type Read = unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32) -> i32;
        let mut out = Vec::new();
        let mut buf = vec![0u8; 1 << 16];
        // SAFETY: a live IStream from GetData; the buffer outlives each call
        unsafe {
            let read: Read = std::mem::transmute(*(*(s as *const *const *const c_void)).add(3));
            loop {
                let mut n = 0u32;
                let hr = read(s, buf.as_mut_ptr() as _, buf.len() as u32, &mut n);
                out.extend_from_slice(&buf[..n as usize]);
                if hr != S_OK || n == 0 {
                    break;
                }
            }
        }
        Some(out)
    }

    // ------------------------------------------------------------ the drop target

    #[repr(C)]
    struct Vtbl {
        query_interface: unsafe extern "system" fn(*mut Target, *const [u8; 16], *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut Target) -> u32,
        release: unsafe extern "system" fn(*mut Target) -> u32,
        drag_enter: unsafe extern "system" fn(*mut Target, *mut c_void, u32, PointL, *mut u32) -> i32,
        drag_over: unsafe extern "system" fn(*mut Target, u32, PointL, *mut u32) -> i32,
        drag_leave: unsafe extern "system" fn(*mut Target) -> i32,
        drop: unsafe extern "system" fn(*mut Target, *mut c_void, u32, PointL, *mut u32) -> i32,
    }

    #[repr(C)]
    struct Target {
        vtbl: *const Vtbl,
        refs: AtomicU32,
        hwnd: isize,
        accept: AtomicBool,
    }

    /// IID bytes as laid out in memory (GUID: u32, u16, u16, [u8; 8]).
    fn iid(d1: u32) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[..4].copy_from_slice(&d1.to_le_bytes());
        b[8] = 0xC0;
        b[15] = 0x46;
        b
    }

    unsafe extern "system" fn query_interface(this: *mut Target, riid: *const [u8; 16], out: *mut *mut c_void) -> i32 {
        // IUnknown {00000000-…-C000-000000000046}, IDropTarget {00000122-…}
        unsafe {
            if *riid == iid(0) || *riid == iid(0x122) {
                *out = this as _;
                add_ref(this);
                S_OK
            } else {
                *out = std::ptr::null_mut();
                E_NOINTERFACE
            }
        }
    }
    unsafe extern "system" fn add_ref(this: *mut Target) -> u32 {
        unsafe { (*this).refs.fetch_add(1, Ordering::Relaxed) + 1 }
    }
    unsafe extern "system" fn release(this: *mut Target) -> u32 {
        // the target lives as long as the window: never freed
        unsafe { (*this).refs.fetch_sub(1, Ordering::Relaxed).saturating_sub(1).max(1) }
    }
    unsafe extern "system" fn drag_enter(this: *mut Target, obj: *mut c_void, _: u32, _: PointL, effect: *mut u32) -> i32 {
        unsafe {
            let ok = !obj.is_null() && Data(obj).usable();
            (*this).accept.store(ok, Ordering::Relaxed);
            *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        }
        S_OK
    }
    unsafe extern "system" fn drag_over(this: *mut Target, _: u32, _: PointL, effect: *mut u32) -> i32 {
        unsafe { *effect = if (*this).accept.load(Ordering::Relaxed) { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        S_OK
    }
    unsafe extern "system" fn drag_leave(this: *mut Target) -> i32 {
        unsafe { (*this).accept.store(false, Ordering::Relaxed) };
        S_OK
    }
    unsafe extern "system" fn drop(this: *mut Target, obj: *mut c_void, _: u32, pt: PointL, effect: *mut u32) -> i32 {
        unsafe {
            *effect = DROPEFFECT_NONE;
            if obj.is_null() {
                return S_OK;
            }
            let mut p = pt;
            ScreenToClient((*this).hwnd, &mut p);
            Data(obj).take(Some([p.x as f32, p.y as f32]));
            *effect = DROPEFFECT_COPY;
        }
        S_OK
    }

    static VTBL: Vtbl = Vtbl { query_interface, add_ref, release, drag_enter, drag_over, drag_leave, drop };

    /// Puts our drop target on the window in place of winit's. winit has
    /// already initialised OLE on this thread (`with_drag_and_drop(true)`).
    pub fn install(handle: &dyn raw_window_handle::HasWindowHandle, ctx: &egui::Context) {
        use raw_window_handle::RawWindowHandle;
        let _ = super::CTX.set(ctx.clone());
        let Ok(h) = handle.window_handle() else { return };
        let RawWindowHandle::Win32(w) = h.as_raw() else { return };
        let hwnd = w.hwnd.get();
        let t = Box::into_raw(Box::new(Target { vtbl: &VTBL, refs: AtomicU32::new(1), hwnd, accept: AtomicBool::new(false) }));
        // SAFETY: our window on its own (OLE) thread; the target is leaked, so
        // it outlives the registration
        unsafe {
            RevokeDragDrop(hwnd);
            let hr = RegisterDragDrop(hwnd, t as _);
            if hr != S_OK {
                eprintln!("drop target: RegisterDragDrop failed 0x{:08X}", hr as u32);
            }
        }
    }

    /// Takes the clipboard's data object as if it were dropped at the cursor
    /// (`DV_DROP_CLIPBOARD=1`: checks the drop path without a mouse).
    pub fn from_clipboard(hwnd_of: &dyn raw_window_handle::HasWindowHandle) {
        use raw_window_handle::RawWindowHandle;
        let mut obj: *mut c_void = std::ptr::null_mut();
        // SAFETY: OleGetClipboard hands back an owned IDataObject (released below)
        unsafe {
            if OleGetClipboard(&mut obj) != S_OK || obj.is_null() {
                return push(Dropped::Failed("clipboard".into()), None);
            }
            let mut p = PointL { x: 0, y: 0 };
            GetCursorPos(&mut p);
            if let Ok(h) = hwnd_of.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    ScreenToClient(w.hwnd.get(), &mut p);
                }
            }
            let d = Data(obj);
            if d.usable() {
                d.take(Some([p.x as f32, p.y as f32]));
            }
            type Release = unsafe extern "system" fn(*mut c_void) -> u32;
            std::mem::transmute::<*const c_void, Release>(d.vt(2))(obj);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A % followed by non-ASCII text stays as it is (before 0.104.2 the
    /// slice between the bytes of a character panicked).
    #[test]
    fn percent_decoding_keeps_text_after_a_percent() {
        assert_eq!(percent_decode("100%한글.png"), "100%한글.png");
        assert_eq!(percent_decode("a%20b%ED%95%9C"), "a b한");
        assert_eq!(percent_decode("%"), "%");
        assert_eq!(percent_decode("%4"), "%4");
        assert_eq!(percent_decode("50%é"), "50%é");
    }

    #[test]
    fn data_urls_and_names() {
        assert_eq!(data_url("data:image/png;base64,iVBORw0KGgo=").unwrap(), vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(data_url("data:text/plain,a%20b").unwrap(), b"a b");
        assert_eq!(name_for("https://x.org/a/cat%20photo.jpg?w=200", b""), "cat photo.jpg");
        assert_eq!(name_for("https://x.org/img?id=3", &[0xFF, 0xD8, 0]), "image.jpg");
        assert_eq!(name_for("data:image/webp;base64,AA", b"RIFF\0\0\0\0WEBP"), "image.webp");
        assert!(is_url("https://a.b/c.png") && is_url(" data:image/png;base64,AA ") && !is_url("hello world") && !is_url("https://a b"));
    }
}
