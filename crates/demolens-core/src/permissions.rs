//! What the operating system lets DemoLens do, for the permissions screen.
//!
//! macOS asks for three things: Screen Recording (required, it is the whole
//! point), Microphone, and Input Monitoring (only to show typed shortcuts).
//! DemoLens needs no Accessibility permission: clicks, the cursor and scroll
//! come from permission free polling in `input.rs`. System audio goes through
//! ScreenCaptureKit and so is covered by Screen Recording.
//!
//! Windows has one switch that matters, "Let desktop apps access your
//! microphone"; Linux has no per app permissions.
//!
//! Everything platform specific is a small function; the list, the "does
//! this need the user's attention" rule and the registry parsing are plain
//! code and tested everywhere.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Granted,
    Denied,
    /// The system has not asked yet.
    Unknown,
    /// Nothing to grant on this system.
    NotNeeded,
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
    pub status: Status,
    pub required: bool,
    /// A note shown when the item is not granted yet.
    pub note: &'static str,
}

pub const SCREEN: &str = "screen";
pub const MIC: &str = "mic";
pub const INPUT: &str = "input";
pub const SYSTEM_AUDIO: &str = "system-audio";

/// The permissions that apply on this system, with their current state.
pub fn list() -> Vec<Item> {
    #[cfg(target_os = "macos")]
    {
        let screen = if mac::screen_granted() { Status::Granted } else { Status::Denied };
        vec![
            Item { id: SCREEN, title: "Screen Recording", detail: "Lets DemoLens see your screen so it can record it.", status: screen, required: true, note: "macOS may ask you to quit and reopen DemoLens after you switch this on." },
            Item { id: MIC, title: "Microphone", detail: "Records your voice with the video.", status: mac::mic_status(), required: false, note: "" },
            Item { id: INPUT, title: "Input Monitoring", detail: "Shows the shortcuts you press (like Cmd+K) in the video. Never what you type.", status: mac::input_status(), required: false, note: "" },
            Item { id: SYSTEM_AUDIO, title: "System audio", detail: "Records what your Mac plays. It uses the Screen Recording permission, and stays off until you switch it on in Record options.", status: screen, required: false, note: "" },
        ]
    }
    #[cfg(windows)]
    {
        vec![Item { id: MIC, title: "Microphone", detail: "Windows has a switch, \"Let desktop apps access your microphone\", that must be on for DemoLens to hear you.", status: win::mic_status(), required: false, note: "" }]
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        vec![Item { id: MIC, title: "Microphone", detail: "Linux has no per app permission. If the microphone is silent, check the input device in your sound settings.", status: Status::NotNeeded, required: false, note: "" }]
    }
}

/// True when the permissions screen should be shown without being asked: a
/// required permission is missing, or (on Windows) the microphone is blocked.
pub fn needs_attention(items: &[Item]) -> bool {
    items.iter().any(|i| (i.required && i.status != Status::Granted) || (i.id == MIC && i.status == Status::Denied && cfg!(windows)))
}

/// Ask for a permission, or open the exact settings page when the system will
/// not ask again. Returns the page opened, if any.
pub fn request(id: &str) -> anyhow::Result<Option<&'static str>> {
    #[cfg(target_os = "macos")]
    {
        let url = settings_url(id);
        match id {
            SCREEN | SYSTEM_AUDIO => {
                // The system prompt is shown once. After that only the settings page helps.
                use std::sync::atomic::{AtomicBool, Ordering};
                static ASKED: AtomicBool = AtomicBool::new(false);
                if !mac::screen_granted() && !ASKED.swap(true, Ordering::SeqCst) {
                    crate::capture::macos_sck::request_permission();
                    return Ok(None);
                }
            }
            MIC => {
                if mac::mic_status() == Status::Unknown {
                    mac::request_mic();
                    return Ok(None);
                }
            }
            INPUT => {
                if mac::input_status() == Status::Unknown {
                    mac::request_input();
                    return Ok(None);
                }
            }
            _ => {}
        }
        if let Some(u) = url {
            open(u)?;
        }
        Ok(url)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let url = settings_url(id);
        if let Some(u) = url {
            open(u)?;
        }
        Ok(url)
    }
}

/// The settings page for a permission on this system.
pub fn settings_url(id: &str) -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        Some(match id {
            SCREEN | SYSTEM_AUDIO => "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
            MIC => "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
            INPUT => "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent",
            _ => return None,
        })
    }
    #[cfg(windows)]
    {
        (id == MIC).then_some("ms-settings:privacy-microphone")
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = id;
        None
    }
}

fn open(url: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        crate::proc::command("open").arg(url).spawn()?;
    }
    #[cfg(windows)]
    {
        crate::proc::command("explorer.exe").arg(url).spawn()?;
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = url;
    }
    Ok(())
}

/// Reads the value of a "ConsentStore" registry entry out of `reg query`
/// output: `Value    REG_SZ    Allow`. `Some(true)` for Allow, `Some(false)`
/// for Deny, `None` when it says neither.
pub fn parse_consent(reg_output: &str) -> Option<bool> {
    for line in reg_output.lines() {
        let mut it = line.split_whitespace();
        if it.next() == Some("Value") {
            let v = it.last()?;
            return match v.to_ascii_lowercase().as_str() {
                "allow" => Some(true),
                "deny" => Some(false),
                _ => None,
            };
        }
    }
    None
}

#[cfg(windows)]
mod win {
    use super::*;
    const BASE: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

    fn read(key: &str) -> Option<bool> {
        let out = crate::proc::command("reg").args(["query", key, "/v", "Value"]).output().ok()?;
        if !out.status.success() {
            return None;
        }
        parse_consent(&String::from_utf8_lossy(&out.stdout))
    }

    /// Both the whole-microphone switch and the "desktop apps" switch must allow it.
    pub fn mic_status() -> Status {
        match (read(BASE), read(&format!(r"{BASE}\NonPackaged"))) {
            (Some(false), _) | (_, Some(false)) => Status::Denied,
            (Some(true), _) | (_, Some(true)) => Status::Granted,
            // Neither key is there: Windows has not been asked, which means allowed.
            _ => Status::Granted,
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::Status;
    use std::ffi::{c_char, c_void};

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOHIDCheckAccess(request: u32) -> u32;
        fn IOHIDRequestAccess(request: u32) -> bool;
    }
    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        static AVMediaTypeAudio: *const c_void;
    }
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> *const c_void;
        fn sel_registerName(name: *const c_char) -> *const c_void;
        fn objc_msgSend();
    }

    const LISTEN_EVENT: u32 = 1;

    pub fn screen_granted() -> bool {
        crate::capture::macos_sck::has_permission()
    }

    /// AVCaptureDevice.authorizationStatus(for: .audio): 0 not determined, 1 restricted, 2 denied, 3 authorized.
    pub fn mic_status() -> Status {
        unsafe {
            let class = objc_getClass(c"AVCaptureDevice".as_ptr());
            if class.is_null() || AVMediaTypeAudio.is_null() {
                return Status::Unknown;
            }
            let sel = sel_registerName(c"authorizationStatusForMediaType:".as_ptr());
            let f: extern "C" fn(*const c_void, *const c_void, *const c_void) -> isize = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            match f(class, sel, AVMediaTypeAudio) {
                3 => Status::Granted,
                0 => Status::Unknown,
                _ => Status::Denied,
            }
        }
    }

    /// Opens the default microphone for a moment, which makes macOS show its
    /// prompt for this app (a request needs no block or callback this way).
    pub fn request_mic() {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        std::thread::spawn(|| {
            let host = cpal::default_host();
            let Some(dev) = host.default_input_device() else { return };
            let Ok(conf) = dev.default_input_config() else { return };
            let stream = match conf.sample_format() {
                cpal::SampleFormat::F32 => dev.build_input_stream::<f32, _, _>(conf.config(), |_: &[f32], _| {}, |_| {}, None),
                cpal::SampleFormat::I16 => dev.build_input_stream::<i16, _, _>(conf.config(), |_: &[i16], _| {}, |_| {}, None),
                _ => return,
            };
            if let Ok(s) = stream {
                let _ = s.play();
                std::thread::sleep(std::time::Duration::from_millis(600));
            }
        });
    }

    /// IOHIDCheckAccess: 0 granted, 1 denied, 2 not asked yet.
    pub fn input_status() -> Status {
        match unsafe { IOHIDCheckAccess(LISTEN_EVENT) } {
            0 => Status::Granted,
            1 => Status::Denied,
            _ => Status::Unknown,
        }
    }

    pub fn request_input() {
        unsafe {
            IOHIDRequestAccess(LISTEN_EVENT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &'static str, status: Status, required: bool) -> Item {
        Item { id, title: "", detail: "", status, required, note: "" }
    }

    #[test]
    fn the_screen_must_be_granted_to_continue_and_nothing_else_forces_the_screen() {
        assert!(needs_attention(&[item(SCREEN, Status::Denied, true), item(MIC, Status::Granted, false)]));
        assert!(needs_attention(&[item(SCREEN, Status::Unknown, true)]));
        assert!(!needs_attention(&[item(SCREEN, Status::Granted, true), item(MIC, Status::Denied, false), item(INPUT, Status::Unknown, false)]) || cfg!(windows));
        assert!(!needs_attention(&[]));
    }

    #[test]
    fn registry_answers_are_read_and_odd_ones_are_not_guessed() {
        let allow = "\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone\n    Value    REG_SZ    Allow\n\n";
        let deny = allow.replace("Allow", "Deny");
        assert_eq!(parse_consent(allow), Some(true));
        assert_eq!(parse_consent(&deny), Some(false));
        assert_eq!(parse_consent("Value    REG_SZ    Prompt"), None);
        assert_eq!(parse_consent("ERROR: The system was unable to find the specified registry key or value."), None);
        assert_eq!(parse_consent(""), None);
    }

    #[test]
    fn the_list_matches_the_system_and_serialises_for_the_page() {
        let l = list();
        assert!(!l.is_empty());
        assert!(l.iter().any(|i| i.id == MIC));
        let json = serde_json::to_string(&l).unwrap();
        assert!(json.contains("\"status\""));
        if cfg!(target_os = "macos") {
            assert!(l.iter().find(|i| i.id == SCREEN).unwrap().required);
            assert!(l.iter().all(|i| i.id != "accessibility"), "no Accessibility permission is used");
            assert!(settings_url(INPUT).unwrap().ends_with("Privacy_ListenEvent"));
        }
        if cfg!(windows) {
            assert_eq!(settings_url(MIC), Some("ms-settings:privacy-microphone"));
        }
    }
}
