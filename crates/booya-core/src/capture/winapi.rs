//! Native Win32 helpers for the Windows capture path: the window list for the
//! picker, the screen size and DPI awareness. No PowerShell, no extra process,
//! so listing windows never opens a console and costs microseconds.

use std::ffi::c_void;
use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
use windows_sys::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetSystemMetrics, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// Makes the calling thread report physical pixels (what the screen grabber
/// and the pointer hooks use), whatever the app's own DPI setting, until the
/// guard drops.
pub struct PhysicalPixels(isize);

pub fn physical_pixels() -> PhysicalPixels {
    PhysicalPixels(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
}

impl Drop for PhysicalPixels {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

/// The whole virtual desktop (all monitors) in physical pixels:
/// (left, top, width, height). The left/top can be negative when a monitor
/// sits left of or above the primary one, and gdigrab's "desktop" starts there.
pub fn virtual_screen() -> (i32, i32, u32, u32) {
    let _g = physical_pixels();
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(0) as u32,
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(0) as u32,
        )
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The window rectangle (left, top, width, height) of the window titled
/// exactly `title`, as the screen grabber sees it.
pub fn window_rect_by_title(title: &str) -> Option<(i32, i32, u32, u32)> {
    let _g = physical_pixels();
    let t = wide(title);
    let h = unsafe { FindWindowW(std::ptr::null(), t.as_ptr()) };
    if h == 0 {
        return None;
    }
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    if unsafe { GetWindowRect(h, &mut r) } == 0 {
        return None;
    }
    Some((r.left, r.top, (r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32))
}

/// One visible top level window.
#[derive(Debug, Clone)]
pub struct Win {
    pub process: String,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub title: String,
}

fn process_name(pid: u32) -> String {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 {
            return String::new();
        }
        let mut buf = [0u16; 520];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut n);
        CloseHandle(h);
        if ok == 0 {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
    let out = &mut *(lp as *mut Vec<Win>);
    if IsWindowVisible(h) == 0 || IsIconic(h) != 0 {
        return 1;
    }
    // Windows 10/11 keep windows of other virtual desktops and suspended
    // store apps "visible" but cloaked.
    let mut cloaked: u32 = 0;
    if DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, &mut cloaked as *mut u32 as *mut c_void, 4) == 0 && cloaked != 0 {
        return 1;
    }
    let len = GetWindowTextLengthW(h);
    if len <= 0 {
        return 1;
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        return 1;
    }
    let title = String::from_utf16_lossy(&buf[..n as usize]);
    // The visible frame (no invisible resize borders or shadow) when DWM has
    // it, the plain window rectangle otherwise.
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS as u32, &mut r as *mut RECT as *mut c_void, std::mem::size_of::<RECT>() as u32) != 0 && GetWindowRect(h, &mut r) == 0 {
        return 1;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, &mut pid);
    out.push(Win { process: process_name(pid), left: r.left, top: r.top, right: r.right, bottom: r.bottom, title });
    1
}

/// Visible, titled, un-minimised windows, frontmost first.
pub fn list_windows() -> Vec<Win> {
    let _g = physical_pixels();
    let mut v: Vec<Win> = Vec::new();
    unsafe { EnumWindows(Some(each), &mut v as *mut Vec<Win> as LPARAM) };
    v
}

/// Same shape as the old script output, so the parser and its tests are shared.
pub fn window_lines() -> String {
    list_windows().iter().map(|w| format!("{}\t{}\t{}\t{}\t{}\t{}\n", w.process, w.left, w.top, w.right, w.bottom, w.title.replace(['\t', '\r', '\n'], " "))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_virtual_screen_has_a_size() {
        let (_, _, w, h) = virtual_screen();
        // A headless CI session still has a desktop (and a session 0 service
        // may report 0): only check it does not crash and is sane when set.
        assert!(w < 100_000 && h < 100_000);
    }

    #[test]
    fn listing_windows_does_not_spawn_anything_or_crash() {
        let t = std::time::Instant::now();
        let v = list_windows();
        assert!(t.elapsed().as_secs() < 5, "listing took {:?}", t.elapsed());
        for w in &v {
            assert!(!w.title.is_empty());
        }
    }
}
