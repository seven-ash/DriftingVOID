// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.

//! winit turns pen input into touch events and drops the pen's button and
//! eraser state, so we peek at WM_POINTER* through a window subclass and keep
//! the flags in globals the app reads each frame.

use std::sync::atomic::{AtomicU32, Ordering};

static PEN_FLAGS: AtomicU32 = AtomicU32::new(0);
static POINTER_FLAGS: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Default, Debug)]
pub struct PenState {
    pub in_range: bool,
    pub barrel: bool,
    pub eraser: bool,
    pub raw_pen: u32,
    pub raw_pointer: u32,
}

pub fn state() -> PenState {
    let pen = PEN_FLAGS.load(Ordering::Relaxed);
    let ptr = POINTER_FLAGS.load(Ordering::Relaxed);
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::Pointer::*;
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        PenState {
            in_range: ptr & POINTER_FLAG_INRANGE != 0,
            barrel: pen & PEN_FLAG_BARREL != 0 || ptr & (POINTER_FLAG_SECONDBUTTON | POINTER_FLAG_THIRDBUTTON) != 0,
            eraser: pen & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0,
            raw_pen: pen,
            raw_pointer: ptr,
        }
    }
    #[cfg(not(windows))]
    PenState { raw_pen: pen, raw_pointer: ptr, ..Default::default() }
}

#[cfg(windows)]
pub fn install(handle: &dyn raw_window_handle::HasWindowHandle) {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    let Ok(h) = handle.window_handle() else { return };
    if let RawWindowHandle::Win32(w) = h.as_raw() {
        // SAFETY: valid HWND owned by this process; the callback only reads
        // pointer info and forwards to DefSubclassProc.
        unsafe {
            SetWindowSubclass(w.hwnd.get() as _, Some(proc_), 0x5D0D, 0);
        }
    }
}

#[cfg(not(windows))]
pub fn install(_: &dyn raw_window_handle::HasWindowHandle) {}

#[cfg(windows)]
unsafe extern "system" fn proc_(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::Input::Pointer::*;
    use windows_sys::Win32::UI::Shell::DefSubclassProc;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    if (WM_POINTERUPDATE..=WM_POINTERLEAVE).contains(&msg) {
        let id = (wparam & 0xFFFF) as u32;
        let mut ty: POINTER_INPUT_TYPE = 0;
        // SAFETY: plain Win32 queries on the pointer id from this message.
        unsafe {
            if GetPointerType(id, &mut ty) != 0 && ty == PT_PEN {
                let mut info: POINTER_PEN_INFO = std::mem::zeroed();
                if GetPointerPenInfo(id, &mut info) != 0 {
                    let mut pf = info.pointerInfo.pointerFlags;
                    if msg == WM_POINTERLEAVE {
                        pf &= !POINTER_FLAG_INRANGE;
                    }
                    PEN_FLAGS.store(info.penFlags, Ordering::Relaxed);
                    POINTER_FLAGS.store(pf, Ordering::Relaxed);
                }
            }
        }
    }
    // SAFETY: forwarding the original message unchanged.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
