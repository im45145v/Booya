//! Finger and pen input.
//!
//! A touch screen tap is not a mouse click to the system: modern apps (the
//! browser, WebView2, GTK3) read touch through their own touch API, so the
//! cursor does not move and no mouse button goes down. The mouse poller in
//! `input.rs` therefore never sees a tap. This module watches the touch
//! digitizer itself:
//!
//! * Windows: Raw Input from HID digitizers (touch screens and pens), parsed
//!   with the HID API, on a message-only window.
//! * Linux (X11): XInput2 raw touch events from the touch screen.
//!
//! Both feed [`Tracker`], which turns "contact N is down at (x, y)" samples
//! into [`Phase`] changes. A tap is logged as `Click` and `Release` with
//! `MouseButton::Touch`, so zoom and the click effects treat it as a click.

use crate::events::{Event, EventKind, MouseButton};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
}

/// One contact in one report: where it is and whether it touches the glass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub id: u32,
    pub down: bool,
    pub x: f64,
    pub y: f64,
}

/// A contact that has not reported for this long has been lifted (its "up"
/// report was lost, or the digitizer only reports changes).
const STALE: Duration = Duration::from_millis(1500);
/// Movements under this many pixels are not worth an event.
const JITTER_PX: f64 = 2.0;

/// Turns samples into down/move/up changes, per contact.
#[derive(Default)]
pub struct Tracker {
    live: HashMap<(i64, u32), (f64, f64, Instant)>,
}

impl Tracker {
    pub fn update(&mut self, now: Instant, dev: i64, samples: &[Sample], out: &mut dyn FnMut(Phase, f64, f64)) {
        for s in samples {
            let key = (dev, s.id);
            if s.down {
                match self.live.get(&key).copied() {
                    None => out(Phase::Down, s.x, s.y),
                    Some((px, py, _)) if ((s.x - px).powi(2) + (s.y - py).powi(2)).sqrt() >= JITTER_PX => out(Phase::Move, s.x, s.y),
                    Some((px, py, _)) => {
                        self.live.insert(key, (px, py, now));
                        continue;
                    }
                }
                self.live.insert(key, (s.x, s.y, now));
            } else if let Some((px, py, _)) = self.live.get(&key).copied() {
                // Hybrid-mode reports pad unused contact slots with zeros: a
                // zero "up" is a filler, not a lift.
                if s.x == 0.0 && s.y == 0.0 && self.live.len() > 1 {
                    continue;
                }
                let (x, y) = if s.x == 0.0 && s.y == 0.0 { (px, py) } else { (s.x, s.y) };
                self.live.remove(&key);
                out(Phase::Up, x, y);
            }
        }
    }

    /// Lifts contacts that went quiet.
    pub fn sweep(&mut self, now: Instant, out: &mut dyn FnMut(Phase, f64, f64)) {
        let stale: Vec<_> = self.live.iter().filter(|(_, v)| now.duration_since(v.2) > STALE).map(|(k, v)| (*k, v.0, v.1)).collect();
        for (k, x, y) in stale {
            self.live.remove(&k);
            out(Phase::Up, x, y);
        }
    }

    pub fn active(&self) -> bool {
        !self.live.is_empty()
    }
}

/// Touch-emulated mouse events (legacy apps get a synthesized left button
/// press from a tap) must not count as a second click: drops the left button
/// press, release and drag from the log while a finger is on the screen.
pub fn drop_emulated_mouse(events: &mut Vec<Event>) {
    // Spans (start, end) in which a finger was down.
    let mut spans: Vec<(f64, f64)> = Vec::new();
    let mut open: Option<f64> = None;
    for e in events.iter() {
        match e.kind {
            EventKind::Click { button: MouseButton::Touch, .. } if open.is_none() => open = Some(e.t),
            EventKind::Release { button: MouseButton::Touch, .. } => {
                if let Some(s) = open.take() {
                    spans.push((s, e.t));
                }
            }
            _ => {}
        }
    }
    if let Some(s) = open {
        spans.push((s, s + 5_000.0));
    }
    if spans.is_empty() {
        return;
    }
    // The system sends the synthesized mouse events a few ms before or after
    // the digitizer's own report, and releases it shortly after the lift.
    let inside = |t: f64| spans.iter().any(|&(a, b)| t >= a - 120.0 && t <= b + 350.0);
    events.retain(|e| match e.kind {
        EventKind::Click { button: MouseButton::Left, .. } | EventKind::Release { button: MouseButton::Left, .. } => !inside(e.t),
        _ => true,
    });
}

/// What the logger gets from the platform watcher.
pub type Sink = Box<dyn FnMut(Phase, f64, f64) + Send>;

/// A running platform watcher. Stops and joins its thread when dropped.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Starts watching the touch screen. `sink` gets screen pixel positions
/// (physical pixels, virtual desktop coordinates). None where touch capture is
/// not available (no digitizer API, no permission, no X11 touch support).
pub fn start(sink: Sink) -> Option<Watcher> {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name("demolens-touch".into())
        .spawn(move || {
            let ok = platform::run(flag, sink, &tx);
            // A platform that never got as far as reporting ready is a no.
            let _ = tx.send(ok);
        })
        .ok()?;
    // The platform code reports once it is registered (true) or has given up.
    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(true) => Some(Watcher { stop, thread: Some(thread) }),
        _ => {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
            None
        }
    }
}

/// Whether the machine has a touch screen, for `demolens doctor`.
pub fn capabilities() -> serde_json::Value {
    platform::capabilities()
}

#[cfg(not(any(windows, all(target_os = "linux", feature = "input-hooks"))))]
mod platform {
    use super::*;
    pub fn run(_stop: Arc<AtomicBool>, _sink: Sink, _ready: &std::sync::mpsc::Sender<bool>) -> bool {
        false
    }
    pub fn capabilities() -> serde_json::Value {
        serde_json::json!({ "supported": serde_json::Value::Null, "note": "touch capture is available on Windows and Linux (X11)" })
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::cell::RefCell;
    use std::ffi::c_void;
    use windows_sys::Win32::Devices::HumanInterfaceDevice::{
        HidP_GetCaps, HidP_GetUsageValue, HidP_GetUsages, HidP_GetValueCaps, HidP_Input, HIDP_CAPS, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS,
    };
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::{
        GetRawInputData, GetRawInputDeviceInfoW, RegisterRawInputDevices, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK, RIDI_PREPARSEDDATA, RID_INPUT, RIM_TYPEHID,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetSystemMetrics, PeekMessageW, RegisterClassExW, TranslateMessage, UnregisterClassW, HWND_MESSAGE, MSG,
        PM_REMOVE, SM_CXSCREEN, SM_CYSCREEN, SM_DIGITIZER, SM_MAXIMUMTOUCHES, WM_INPUT, WNDCLASSEXW,
    };

    const PAGE_GENERIC: u16 = 0x01;
    const PAGE_DIGITIZER: u16 = 0x0D;
    const USAGE_X: u16 = 0x30;
    const USAGE_Y: u16 = 0x31;
    const USAGE_TIP: u16 = 0x42;
    const USAGE_CONTACT_ID: u16 = 0x51;

    /// One contact slot of a digitizer: its HID link collection and the range
    /// of its coordinates.
    struct Slot {
        link: u16,
        x: (f64, f64),
        y: (f64, f64),
        has_id: bool,
    }

    struct Dev {
        /// Preparsed data, 8 byte aligned as the HID API wants.
        pre: Vec<u64>,
        slots: Vec<Slot>,
    }

    struct State {
        devs: std::collections::HashMap<isize, Option<Dev>>,
        tracker: Tracker,
        sink: Sink,
        screen: (f64, f64),
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn has_usage(c: &HIDP_VALUE_CAPS, page: u16, usage: u16) -> bool {
        if c.UsagePage != page {
            return false;
        }
        unsafe {
            if c.IsRange != 0 {
                (c.Anonymous.Range.UsageMin..=c.Anonymous.Range.UsageMax).contains(&usage)
            } else {
                c.Anonymous.NotRange.Usage == usage
            }
        }
    }

    unsafe fn load_dev(h: isize) -> Option<Dev> {
        let mut n = 0u32;
        GetRawInputDeviceInfoW(h, RIDI_PREPARSEDDATA, std::ptr::null_mut(), &mut n);
        if n == 0 {
            return None;
        }
        let mut pre = vec![0u64; (n as usize).div_ceil(8)];
        if GetRawInputDeviceInfoW(h, RIDI_PREPARSEDDATA, pre.as_mut_ptr() as *mut c_void, &mut n) == u32::MAX {
            return None;
        }
        let pp = pre.as_ptr() as isize;
        let mut caps: HIDP_CAPS = std::mem::zeroed();
        if HidP_GetCaps(pp, &mut caps) != HIDP_STATUS_SUCCESS {
            return None;
        }
        let mut len = caps.NumberInputValueCaps;
        if len == 0 {
            return None;
        }
        let mut vc: Vec<HIDP_VALUE_CAPS> = vec![std::mem::zeroed(); len as usize];
        if HidP_GetValueCaps(HidP_Input, vc.as_mut_ptr(), &mut len, pp) != HIDP_STATUS_SUCCESS {
            return None;
        }
        vc.truncate(len as usize);
        let mut slots: Vec<Slot> = Vec::new();
        for c in &vc {
            let (is_x, is_y) = (has_usage(c, PAGE_GENERIC, USAGE_X), has_usage(c, PAGE_GENERIC, USAGE_Y));
            if !is_x && !is_y {
                continue;
            }
            let range = (c.LogicalMin as f64, c.LogicalMax as f64);
            let i = match slots.iter().position(|s| s.link == c.LinkCollection) {
                Some(i) => i,
                None => {
                    slots.push(Slot { link: c.LinkCollection, x: (0.0, 0.0), y: (0.0, 0.0), has_id: false });
                    slots.len() - 1
                }
            };
            if is_x {
                slots[i].x = range;
            }
            if is_y {
                slots[i].y = range;
            }
        }
        for s in &mut slots {
            s.has_id = vc.iter().any(|c| c.LinkCollection == s.link && has_usage(c, PAGE_DIGITIZER, USAGE_CONTACT_ID));
        }
        slots.retain(|s| s.x.1 > s.x.0 && s.y.1 > s.y.0);
        if slots.is_empty() {
            return None;
        }
        Some(Dev { pre, slots })
    }

    fn norm(v: f64, (lo, hi): (f64, f64)) -> f64 {
        ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
    }

    unsafe fn on_input(st: &mut State, lparam: LPARAM) {
        let head = std::mem::size_of::<RAWINPUTHEADER>() as u32;
        let mut size = 0u32;
        if GetRawInputData(lparam, RID_INPUT, std::ptr::null_mut(), &mut size, head) != 0 || size == 0 {
            return;
        }
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        if GetRawInputData(lparam, RID_INPUT, buf.as_mut_ptr() as *mut c_void, &mut size, head) != size {
            return;
        }
        let ri = &*(buf.as_ptr() as *const RAWINPUT);
        if ri.header.dwType != RIM_TYPEHID {
            return;
        }
        let dev_handle = ri.header.hDevice;
        let hid = std::ptr::addr_of!(ri.data.hid);
        let (size_hid, count) = ((*hid).dwSizeHid as usize, (*hid).dwCount as usize);
        let data = std::ptr::addr_of!((*hid).bRawData) as *const u8;
        let dev = st.devs.entry(dev_handle).or_insert_with(|| load_dev(dev_handle));
        let Some(dev) = dev.as_ref() else { return };
        let pp = dev.pre.as_ptr() as isize;
        let now = Instant::now();
        for i in 0..count {
            let mut report = std::slice::from_raw_parts(data.add(i * size_hid), size_hid).to_vec();
            let mut samples: Vec<Sample> = Vec::new();
            for slot in &dev.slots {
                let mut usages = [0u16; 16];
                let mut n = usages.len() as u32;
                let ok = HidP_GetUsages(HidP_Input, PAGE_DIGITIZER, slot.link, usages.as_mut_ptr(), &mut n, pp, report.as_mut_ptr(), report.len() as u32);
                if ok != HIDP_STATUS_SUCCESS {
                    continue; // this report is for another report id
                }
                let down = usages[..(n as usize).min(16)].contains(&USAGE_TIP);
                let (mut x, mut y, mut id) = (0u32, 0u32, slot.link as u32);
                let rp = report.as_ptr();
                let rl = report.len() as u32;
                if HidP_GetUsageValue(HidP_Input, PAGE_GENERIC, slot.link, USAGE_X, &mut x, pp, rp, rl) != HIDP_STATUS_SUCCESS
                    || HidP_GetUsageValue(HidP_Input, PAGE_GENERIC, slot.link, USAGE_Y, &mut y, pp, rp, rl) != HIDP_STATUS_SUCCESS
                {
                    continue;
                }
                if slot.has_id {
                    let mut v = 0u32;
                    if HidP_GetUsageValue(HidP_Input, PAGE_DIGITIZER, slot.link, USAGE_CONTACT_ID, &mut v, pp, rp, rl) == HIDP_STATUS_SUCCESS {
                        id = v;
                    }
                }
                let (px, py) = if x == 0 && y == 0 { (0.0, 0.0) } else { (norm(x as f64, slot.x) * st.screen.0, norm(y as f64, slot.y) * st.screen.1) };
                samples.push(Sample { id, down, x: px, y: py });
            }
            let sink = &mut st.sink;
            st.tracker.update(now, dev_handle as i64, &samples, &mut |p, x, y| sink(p, x, y));
        }
    }

    unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if m == WM_INPUT {
            STATE.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    on_input(st, l);
                }
            });
        }
        DefWindowProcW(h, m, w, l)
    }

    pub fn run(stop: Arc<AtomicBool>, sink: Sink, ready: &std::sync::mpsc::Sender<bool>) -> bool {
        // Physical pixels, so positions line up with the screen grabber.
        let _px = crate::capture::winapi::physical_pixels();
        unsafe {
            let class = wide("DemoLensTouchSink");
            let hinst = GetModuleHandleW(std::ptr::null());
            let mut wc: WNDCLASSEXW = std::mem::zeroed();
            wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
            wc.lpfnWndProc = Some(wndproc);
            wc.hInstance = hinst;
            wc.lpszClassName = class.as_ptr();
            RegisterClassExW(&wc);
            let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, HWND_MESSAGE, 0, hinst, std::ptr::null());
            if hwnd == 0 {
                UnregisterClassW(class.as_ptr(), hinst);
                return false;
            }
            // Touch screens (usage 4) and pens (usage 2) on the digitizer page.
            // Not touch pads (usage 5): a trackpad tap is a mouse click.
            let mut any = false;
            for usage in [0x04u16, 0x02] {
                let rid = RAWINPUTDEVICE { usUsagePage: PAGE_DIGITIZER, usUsage: usage, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
                any |= RegisterRawInputDevices(&rid, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32) != 0;
            }
            if !any {
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), hinst);
                return false;
            }
            let screen = (GetSystemMetrics(SM_CXSCREEN).max(1) as f64, GetSystemMetrics(SM_CYSCREEN).max(1) as f64);
            STATE.with(|s| *s.borrow_mut() = Some(State { devs: Default::default(), tracker: Tracker::default(), sink, screen }));
            let _ = ready.send(true);
            let mut msg: MSG = std::mem::zeroed();
            while !stop.load(Ordering::Relaxed) {
                while PeekMessageW(&mut msg, 0, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                STATE.with(|s| {
                    if let Some(st) = s.borrow_mut().as_mut() {
                        let sink = &mut st.sink;
                        st.tracker.sweep(Instant::now(), &mut |p, x, y| sink(p, x, y));
                    }
                });
                std::thread::sleep(Duration::from_millis(4));
            }
            STATE.with(|s| *s.borrow_mut() = None);
            DestroyWindow(hwnd);
            UnregisterClassW(class.as_ptr(), hinst);
        }
        // Already reported ready; the value is not used again.
        true
    }

    pub fn capabilities() -> serde_json::Value {
        let d = unsafe { GetSystemMetrics(SM_DIGITIZER) };
        let max = unsafe { GetSystemMetrics(SM_MAXIMUMTOUCHES) };
        serde_json::json!({
            "supported": d & 0x03 != 0,
            "integrated_touch": d & 0x01 != 0,
            "external_touch": d & 0x02 != 0,
            "ready": d & 0x80 != 0,
            "max_contacts": max,
            "backend": "Raw Input (HID digitizer)",
        })
    }
}

#[cfg(all(target_os = "linux", feature = "input-hooks"))]
mod platform {
    use super::*;
    use std::ffi::{CStr, CString};
    use x11::{xinput2 as xi, xlib};

    /// Axis numbers and ranges of one touch device.
    struct Axes {
        x: (i32, f64, f64),
        y: (i32, f64, f64),
    }

    unsafe fn atom_name(dpy: *mut xlib::Display, a: xlib::Atom) -> String {
        if a == 0 {
            return String::new();
        }
        let p = xlib::XGetAtomName(dpy, a);
        if p.is_null() {
            return String::new();
        }
        let s = CStr::from_ptr(p).to_string_lossy().into_owned();
        xlib::XFree(p as *mut _);
        s
    }

    /// The pointer axes of every device that has them, found by label
    /// ("Abs MT Position X" on touch screens, "Abs X" on older drivers).
    unsafe fn axes_of_devices(dpy: *mut xlib::Display) -> std::collections::HashMap<i32, Axes> {
        let mut out = std::collections::HashMap::new();
        let mut n = 0;
        let info = xi::XIQueryDevice(dpy, xi::XIAllDevices, &mut n);
        if info.is_null() {
            return out;
        }
        for i in 0..n as isize {
            let d = &*info.offset(i);
            let (mut x, mut y, mut fx, mut fy) = (None, None, None, None);
            for c in 0..d.num_classes as isize {
                let cl = *d.classes.offset(c);
                if (*cl)._type != xi::XIValuatorClass {
                    continue;
                }
                let v = &*(cl as *const xi::XIValuatorClassInfo);
                let label = atom_name(dpy, v.label);
                let t = (v.number, v.min, v.max);
                if label.contains("MT Position X") {
                    x = Some(t);
                } else if label.contains("MT Position Y") {
                    y = Some(t);
                } else if label == "Abs X" {
                    fx = Some(t);
                } else if label == "Abs Y" {
                    fy = Some(t);
                }
            }
            if let (Some(x), Some(y)) = (x.or(fx), y.or(fy)) {
                out.insert(d.deviceid, Axes { x, y });
            }
        }
        xi::XIFreeDeviceInfo(info);
        out
    }

    unsafe fn raw_value(ev: &xi::XIRawEvent, axis: i32) -> Option<f64> {
        let mask = std::slice::from_raw_parts(ev.valuators.mask, ev.valuators.mask_len as usize);
        let set = |n: i32| (n as usize) / 8 < mask.len() && mask[(n as usize) / 8] & (1 << (n % 8)) != 0;
        if !set(axis) {
            return None;
        }
        let idx = (0..axis).filter(|&n| set(n)).count();
        Some(*ev.raw_values.add(idx))
    }

    pub fn run(stop: Arc<AtomicBool>, mut sink: Sink, ready: &std::sync::mpsc::Sender<bool>) -> bool {
        unsafe {
            let dpy = xlib::XOpenDisplay(std::ptr::null());
            if dpy.is_null() {
                return false;
            }
            let ext = CString::new("XInputExtension").unwrap();
            let (mut opcode, mut ev, mut err) = (0, 0, 0);
            if xlib::XQueryExtension(dpy, ext.as_ptr(), &mut opcode, &mut ev, &mut err) == 0 {
                xlib::XCloseDisplay(dpy);
                return false;
            }
            // Touch needs XInput 2.2.
            let (mut major, mut minor) = (2, 2);
            if xi::XIQueryVersion(dpy, &mut major, &mut minor) != 0 || (major, minor) < (2, 2) {
                xlib::XCloseDisplay(dpy);
                return false;
            }
            let axes = axes_of_devices(dpy);
            if axes.is_empty() {
                xlib::XCloseDisplay(dpy);
                return false;
            }
            let root = xlib::XDefaultRootWindow(dpy);
            let mut mask = [0u8; 4];
            for e in [xi::XI_RawTouchBegin, xi::XI_RawTouchUpdate, xi::XI_RawTouchEnd] {
                xi::XISetMask(&mut mask, e);
            }
            let mut em = xi::XIEventMask { deviceid: xi::XIAllMasterDevices, mask_len: mask.len() as i32, mask: mask.as_mut_ptr() };
            xi::XISelectEvents(dpy, root, &mut em, 1);
            xlib::XFlush(dpy);
            let screen = xlib::XDefaultScreen(dpy);
            let (sw, sh) = (xlib::XDisplayWidth(dpy, screen) as f64, xlib::XDisplayHeight(dpy, screen) as f64);
            let _ = ready.send(true);
            let mut tracker = Tracker::default();
            while !stop.load(Ordering::Relaxed) {
                while xlib::XPending(dpy) > 0 {
                    let mut e: xlib::XEvent = std::mem::zeroed();
                    xlib::XNextEvent(dpy, &mut e);
                    if e.get_type() != xlib::GenericEvent || e.generic_event_cookie.extension != opcode || xlib::XGetEventData(dpy, &mut e.generic_event_cookie) == 0 {
                        continue;
                    }
                    let cookie = &mut e.generic_event_cookie;
                    let raw = &*(cookie.data as *const xi::XIRawEvent);
                    let evtype = cookie.evtype;
                    if let Some(a) = axes.get(&raw.sourceid) {
                        if let (Some(rx), Some(ry)) = (raw_value(raw, a.x.0), raw_value(raw, a.y.0)) {
                            let norm = |v: f64, lo: f64, hi: f64| if hi > lo { ((v - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
                            let s = Sample { id: raw.detail as u32, down: evtype != xi::XI_RawTouchEnd, x: norm(rx, a.x.1, a.x.2) * sw, y: norm(ry, a.y.1, a.y.2) * sh };
                            tracker.update(Instant::now(), raw.sourceid as i64, &[s], &mut |p, x, y| sink(p, x, y));
                        }
                    }
                    xlib::XFreeEventData(dpy, &mut e.generic_event_cookie);
                }
                tracker.sweep(Instant::now(), &mut |p, x, y| sink(p, x, y));
                std::thread::sleep(Duration::from_millis(4));
            }
            xlib::XCloseDisplay(dpy);
        }
        true
    }

    pub fn capabilities() -> serde_json::Value {
        unsafe {
            let dpy = xlib::XOpenDisplay(std::ptr::null());
            if dpy.is_null() {
                return serde_json::json!({ "supported": null, "note": "no X11 display" });
            }
            let n = axes_of_devices(dpy).len();
            xlib::XCloseDisplay(dpy);
            serde_json::json!({ "supported": n > 0, "pointer_devices_with_axes": n, "backend": "XInput2 raw touch events", "note": "Wayland sessions are not supported yet" })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &mut Tracker, now: Instant, s: &[Sample]) -> Vec<(Phase, i64, i64)> {
        let mut v = Vec::new();
        t.update(now, 1, s, &mut |p, x, y| v.push((p, x as i64, y as i64)));
        v
    }

    fn s(id: u32, down: bool, x: f64, y: f64) -> Sample {
        Sample { id, down, x, y }
    }

    #[test]
    fn a_tap_is_down_then_up() {
        let mut t = Tracker::default();
        let now = Instant::now();
        assert_eq!(run(&mut t, now, &[s(1, true, 100.0, 200.0)]), vec![(Phase::Down, 100, 200)]);
        // Same place: no move event for jitter.
        assert!(run(&mut t, now, &[s(1, true, 101.0, 200.0)]).is_empty());
        assert_eq!(run(&mut t, now, &[s(1, false, 101.0, 200.0)]), vec![(Phase::Up, 101, 200)]);
        assert!(!t.active());
    }

    #[test]
    fn a_drag_moves_and_two_fingers_are_two_contacts() {
        let mut t = Tracker::default();
        let now = Instant::now();
        run(&mut t, now, &[s(1, true, 10.0, 10.0), s(2, true, 500.0, 500.0)]);
        assert_eq!(run(&mut t, now, &[s(1, true, 60.0, 10.0)]), vec![(Phase::Move, 60, 10)]);
        assert_eq!(run(&mut t, now, &[s(2, false, 500.0, 500.0)]), vec![(Phase::Up, 500, 500)]);
        assert!(t.active());
    }

    #[test]
    fn zero_filled_slots_do_not_lift_a_real_finger() {
        let mut t = Tracker::default();
        let now = Instant::now();
        run(&mut t, now, &[s(0, true, 300.0, 300.0), s(1, true, 400.0, 400.0)]);
        // Contact 0 shows up as an all zero filler slot while 1 is still down.
        assert!(run(&mut t, now, &[s(0, false, 0.0, 0.0), s(1, true, 400.0, 400.0)]).is_empty());
        assert!(t.active());
    }

    #[test]
    fn a_lost_up_report_is_swept() {
        let mut t = Tracker::default();
        let now = Instant::now();
        run(&mut t, now, &[s(3, true, 50.0, 60.0)]);
        let mut v = Vec::new();
        t.sweep(now + Duration::from_millis(500), &mut |p, x, y| v.push((p, x as i64, y as i64)));
        assert!(v.is_empty());
        t.sweep(now + Duration::from_secs(3), &mut |p, x, y| v.push((p, x as i64, y as i64)));
        assert_eq!(v, vec![(Phase::Up, 50, 60)]);
    }

    fn ev(t: f64, kind: EventKind) -> Event {
        Event { t, kind }
    }

    #[test]
    fn the_mouse_click_a_tap_causes_is_dropped_but_real_clicks_stay() {
        let left = |x: f64| EventKind::Click { x, y: 5.0, button: MouseButton::Left };
        let left_up = |x: f64| EventKind::Release { x, y: 5.0, button: MouseButton::Left };
        let mut events = vec![
            ev(100.0, left(1.0)),
            ev(180.0, left_up(1.0)),
            ev(1000.0, EventKind::Click { x: 50.0, y: 5.0, button: MouseButton::Touch }),
            ev(1010.0, left(50.0)),
            ev(1100.0, EventKind::Release { x: 50.0, y: 5.0, button: MouseButton::Touch }),
            ev(1120.0, left_up(50.0)),
            ev(3000.0, left(9.0)),
        ];
        drop_emulated_mouse(&mut events);
        let lefts: Vec<f64> = events.iter().filter(|e| matches!(e.kind, EventKind::Click { button: MouseButton::Left, .. })).map(|e| e.t).collect();
        assert_eq!(lefts, vec![100.0, 3000.0]);
        assert_eq!(events.iter().filter(|e| matches!(e.kind, EventKind::Click { button: MouseButton::Touch, .. })).count(), 1);
        assert!(events.iter().all(|e| e.t != 1120.0));
    }

    /// Starting and stopping the watcher must never hang or crash, with or
    /// without a touch screen (CI has none, so it may legitimately be None).
    #[test]
    fn the_watcher_starts_and_stops() {
        let t = Instant::now();
        let w = start(Box::new(|_, _, _| {}));
        drop(w);
        assert!(t.elapsed() < Duration::from_secs(10));
        let _ = capabilities();
    }
}
