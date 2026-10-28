//! Microphone devices and the input level meter.

use anyhow::{anyhow, Result};
#[cfg(not(windows))]
use std::io::Read;
#[cfg(not(windows))]
use std::process::Stdio;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Names of the microphones, built-in first. May be empty when the OS lists
/// none (or when listing is not supported, in which case the default is used).
pub fn list_mics() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        let (_, audio) = super::macos::list_devices();
        let mut names: Vec<String> = audio.into_iter().map(|d| d.name).collect();
        names.sort_by_key(|n| !is_builtin(n));
        names
    }
    #[cfg(target_os = "linux")]
    {
        let out = crate::proc::command("pactl").args(["list", "short", "sources"]).output();
        out.map(|o| parse_pactl_sources(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
    }
    #[cfg(target_os = "windows")]
    {
        // The names Windows shows (default first), which is also what the
        // recorder opens. DirectShow's list is only the fallback.
        let v = super::wasapi::input_names();
        if !v.is_empty() {
            return v;
        }
        dshow_devices().into_iter().map(|d| d.name).collect()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Vec::new()
    }
}

/// DirectShow audio devices, from ffmpeg.
#[cfg(target_os = "windows")]
pub(crate) fn dshow_devices() -> Vec<DshowDevice> {
    let out = crate::proc::command(crate::ffmpeg::ffmpeg_bin()).args(["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"]).output();
    out.map(|o| parse_dshow_audio_devices(&String::from_utf8_lossy(&o.stderr))).unwrap_or_default()
}

/// A DirectShow audio device: the friendly name and the "alternative name"
/// (a device path that is plain ASCII, so it survives any quoting or code
/// page problem that a name with accents or symbols can hit).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DshowDevice {
    pub name: String,
    pub alt: Option<String>,
}

/// Parses `ffmpeg -list_devices true -f dshow -i dummy`, pairing each audio
/// device with the "Alternative name" line that follows it.
#[allow(dead_code)]
pub(crate) fn parse_dshow_audio_devices(text: &str) -> Vec<DshowDevice> {
    let mut v: Vec<DshowDevice> = Vec::new();
    let mut in_audio_section = false;
    let mut last_is_audio = false;
    for l in text.lines() {
        let quoted = || {
            let a = l.find('"')? + 1;
            let b = l.rfind('"')?;
            (b > a).then(|| l[a..b].to_string())
        };
        if l.contains("DirectShow audio devices") {
            in_audio_section = true;
            continue;
        }
        if l.contains("DirectShow video devices") {
            in_audio_section = false;
            last_is_audio = false;
            continue;
        }
        if l.contains("Alternative name") {
            if last_is_audio {
                if let (Some(d), Some(alt)) = (v.last_mut(), quoted()) {
                    d.alt = Some(alt);
                }
            }
            continue;
        }
        if l.contains("(audio)") || (in_audio_section && !l.contains("(video)") && !l.contains("(none)")) {
            if let Some(n) = quoted() {
                v.push(DshowDevice { name: n, alt: None });
                last_is_audio = true;
                continue;
            }
        }
        last_is_audio = false;
    }
    v
}

#[allow(dead_code)]
pub(crate) fn is_builtin(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("macbook") || n.contains("built-in") || n.contains("internal") || n.contains("imac") || n.contains("mac mini") || n.contains("mac studio") || n.contains("studio display")
}

/// Pulse sources that are real inputs (monitors of speakers are not).
#[allow(dead_code)]
pub(crate) fn parse_pactl_sources(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .filter(|n| !n.ends_with(".monitor"))
        .map(str::to_string)
        .collect()
}

#[allow(dead_code)]
pub(crate) fn parse_dshow_audio(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.contains("(audio)"))
        .filter_map(|l| {
            let a = l.find('"')? + 1;
            let b = l[a..].find('"')? + a;
            Some(l[a..b].to_string())
        })
        .collect()
}

/// Video devices (cameras) in the output of `ffmpeg -list_devices true -f dshow`.
/// Newer ffmpeg builds print `"Name" (video)`, older ones a line with the
/// name and then one saying `Alternative name`; both are handled.
#[allow(dead_code)]
pub(crate) fn parse_dshow_video(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.contains("(video)"))
        .filter_map(|l| {
            let a = l.find('"')? + 1;
            let b = l[a..].find('"')? + a;
            Some(l[a..b].to_string())
        })
        .collect()
}

/// The camera to record: the one named by the user, else the first one that
/// is a real camera (not a virtual one made by streaming software or a
/// screen capture driver).
#[allow(dead_code)]
pub(crate) fn pick_camera(names: &[String], wanted: Option<&str>) -> Option<String> {
    if let Some(w) = wanted.filter(|w| !w.trim().is_empty()) {
        if let Some(n) = names.iter().find(|n| n.eq_ignore_ascii_case(w.trim())) {
            return Some(n.clone());
        }
    }
    let virt = |n: &str| {
        let n = n.to_lowercase();
        ["virtual", "obs", "snap camera", "manycam", "screen capture", "droidcam", "ndi ", "xsplit", "camo"].iter().any(|v| n.contains(v))
    };
    names.iter().find(|n| !virt(n)).or_else(|| names.first()).cloned()
}

/// ffmpeg input arguments for a microphone (`name` None = the default one).
pub(crate) fn input_args(name: Option<&str>) -> Result<Vec<String>> {
    #[cfg(target_os = "macos")]
    {
        let (_, audio) = super::macos::list_devices();
        let d = super::macos::pick_mic(&audio, name).ok_or_else(|| anyhow!("No microphone was found."))?;
        Ok(vec!["-f".into(), "avfoundation".into(), "-i".into(), format!(":{}", d.index)])
    }
    #[cfg(target_os = "linux")]
    {
        Ok(vec!["-f".into(), "pulse".into(), "-i".into(), name.unwrap_or("default").to_string()])
    }
    #[cfg(target_os = "windows")]
    {
        // DirectShow lists devices in its own order, which is not the Windows
        // default, so the choice is made against the list ffmpeg itself sees.
        let devs = dshow_devices();
        let names: Vec<String> = devs.iter().map(|d| d.name.clone()).collect();
        let want = name.map(str::to_string).or_else(|| std::env::var("DEMOLENS_MIC").ok());
        let default = super::wasapi::input_names().into_iter().next();
        let dev = super::mic::pick_device_name(&names, want.as_deref(), default.as_deref()).ok_or_else(|| anyhow!("DirectShow lists no microphone"))?;
        let entry = devs.iter().find(|d| d.name == dev);
        // A name with accents or symbols is passed by its device path.
        let target = match entry.and_then(|d| d.alt.clone()) {
            Some(alt) if !dev.is_ascii() || dev.contains('"') => alt,
            _ => dev,
        };
        Ok(vec!["-f".into(), "dshow".into(), "-i".into(), format!("audio={target}")])
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = name;
        Err(anyhow!("microphone capture is not supported on this platform"))
    }
}

/// RMS and peak of 16 bit samples, in dBFS (floor -90).
pub fn levels_db(samples: &[i16]) -> (f32, f32) {
    if samples.is_empty() {
        return (-90.0, -90.0);
    }
    let mut sum = 0.0f64;
    let mut peak = 0i32;
    for &s in samples {
        let v = s as f64 / 32768.0;
        sum += v * v;
        peak = peak.max((s as i32).abs());
    }
    let rms = (sum / samples.len() as f64).sqrt();
    let db = |v: f64| if v <= 1e-5 { -90.0 } else { (20.0 * v.log10()) as f32 };
    (db(rms), db(peak as f64 / 32768.0))
}

/// Reads the microphone and reports a level about every 100 ms until dropped
/// or stopped. Used for the meter before and during a recording.
pub struct MicMeter {
    child: Option<Child>,
    stop: Arc<AtomicBool>,
    #[cfg(windows)]
    _wasapi: Option<super::wasapi::Recorder>,
}

impl MicMeter {
    /// `on_level(rms_db, peak_db)` is called from a background thread.
    pub fn start(name: Option<&str>, on_level: impl Fn(f32, f32) + Send + 'static) -> Result<Self> {
        #[cfg(windows)]
        {
            // In process, the same way the recording opens the device.
            let on_level = std::sync::Mutex::new(on_level);
            match super::wasapi::meter(name.map(str::to_string), move |r, p| (on_level.lock().unwrap())(r, p)) {
                Ok(w) => return Ok(Self { child: None, stop: Arc::new(AtomicBool::new(false)), _wasapi: Some(w) }),
                Err(_) => return Err(anyhow!("the microphone could not be opened for the level meter")),
            }
        }
        #[cfg(not(windows))]
        Self::start_ffmpeg(name, on_level)
    }

    #[cfg(not(windows))]
    fn start_ffmpeg(name: Option<&str>, on_level: impl Fn(f32, f32) + Send + 'static) -> Result<Self> {
        let input = input_args(name)?;
        let mut cmd = crate::proc::command(crate::ffmpeg::ffmpeg_bin());
        cmd.args(["-hide_banner", "-nostats", "-v", "error", "-nostdin"]).args(&input);
        cmd.args(["-ac", "1", "-ar", "16000", "-f", "s16le", "-"]);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd.spawn().map_err(|e| anyhow!("could not start ffmpeg for the microphone meter: {e}"))?;
        let mut out = child.stdout.take().ok_or_else(|| anyhow!("no meter output"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::Builder::new()
            .name("demolens-mic-meter".into())
            .spawn(move || {
                let mut buf = vec![0u8; 3200]; // 100 ms of 16 kHz mono 16 bit
                while !flag.load(Ordering::Relaxed) {
                    if out.read_exact(&mut buf).is_err() {
                        break;
                    }
                    let samples: Vec<i16> = buf.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
                    let (rms, peak) = levels_db(&samples);
                    on_level(rms, peak);
                }
            })?;
        Ok(Self { child: Some(child), stop })
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(c) = self.child.as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        #[cfg(windows)]
        {
            // Dropping the recorder stops the stream and frees the device.
            self._wasapi = None;
        }
    }
}

impl Drop for MicMeter {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dshow_devices_are_paired_with_their_alternative_names() {
        let text = r#"[dshow @ 000001] DirectShow video devices (some may be both video and audio devices)
[dshow @ 000001]  "Integrated Camera" (video)
[dshow @ 000001]   Alternative name "@device_pnp_\\?\usb#vid_04f2"
[dshow @ 000001] DirectShow audio devices
[dshow @ 000001]  "Mikrofon (Realtek(R) Audio) é" (audio)
[dshow @ 000001]   Alternative name "@device_cm_{33D9A762}\wave_{AAA}"
[dshow @ 000001]  "Headset (USB)" (audio)
[dshow @ 000001]   Alternative name "@device_cm_{33D9A762}\wave_{BBB}"
"#;
        let d = super::parse_dshow_audio_devices(text);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].name, "Mikrofon (Realtek(R) Audio) é");
        assert!(d[0].alt.as_deref().unwrap().starts_with("@device_cm_"));
        assert!(d[1].alt.as_deref().unwrap().ends_with("wave_{BBB}"));
    }

    use super::*;

    #[test]
    fn levels_of_silence_and_a_loud_tone() {
        assert_eq!(levels_db(&[0; 100]), (-90.0, -90.0));
        let full: Vec<i16> = (0..1000).map(|i| if i % 2 == 0 { 32767 } else { -32767 }).collect();
        let (rms, peak) = levels_db(&full);
        assert!(rms > -0.1 && peak > -0.1);
        let quiet: Vec<i16> = (0..1000).map(|i| if i % 2 == 0 { 100 } else { -100 }).collect();
        let (rms, _) = levels_db(&quiet);
        assert!((rms + 50.3).abs() < 0.5, "{rms}");
    }

    #[test]
    fn parses_device_lists() {
        let p = parse_pactl_sources("0\talsa_output.pci.monitor\tmodule\ts16le\tIDLE\n1\talsa_input.usb-Mic\tmodule\ts16le\tIDLE\n");
        assert_eq!(p, vec!["alsa_input.usb-Mic"]);
        let w = parse_dshow_audio("[dshow @ 0x1] \"Microphone (Realtek)\" (audio)\n[dshow @ 0x1] \"Integrated Camera\" (video)\n");
        assert_eq!(w, vec!["Microphone (Realtek)"]);
        let cams = parse_dshow_video("[dshow @ 0x1] \"OBS Virtual Camera\" (video)\n[dshow @ 0x1]   Alternative name \"@device_sw_x\"\n[dshow @ 0x1] \"Integrated Camera\" (video)\n[dshow @ 0x1] \"Microphone (Realtek)\" (audio)\n");
        assert_eq!(cams, vec!["OBS Virtual Camera", "Integrated Camera"]);
        assert_eq!(pick_camera(&cams, None).as_deref(), Some("Integrated Camera"), "a virtual camera is not the default");
        assert_eq!(pick_camera(&cams, Some("obs virtual camera")).as_deref(), Some("OBS Virtual Camera"), "unless asked for");
        assert_eq!(pick_camera(&cams[..1], None).as_deref(), Some("OBS Virtual Camera"), "better than nothing");
        assert_eq!(pick_camera(&[], None), None);
        assert!(is_builtin("MacBook Air Microphone"));
        assert!(!is_builtin("WH-CH520"));
    }
}
