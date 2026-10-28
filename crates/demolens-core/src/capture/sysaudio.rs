//! System audio (what the computer plays) as a sidecar, like the microphone:
//! recorded on the shared recording clock into `recording.sys.mkv`, then mixed
//! into the microphone file by `finalize::merge_system_audio` when the
//! recording stops, so the remux only ever sees one audio file.
//!
//! * Windows: WASAPI loopback of the default speakers, through cpal.
//! * macOS: a ScreenCaptureKit audio stream (macOS 13 or newer).
//! * Linux: not a sidecar; the screen ffmpeg records the pulse monitor.
//!
//! It is off unless the user switches "System audio" on.

use std::path::{Path, PathBuf};

/// Whether this OS records system audio as a sidecar.
pub(crate) fn supported() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

pub(crate) struct SysAudio {
    inner: Option<Inner>,
    out: PathBuf,
    log: PathBuf,
}

enum Inner {
    #[cfg(windows)]
    Wasapi(super::wasapi::Recorder),
    #[cfg(target_os = "macos")]
    Sck(super::macos_sck::AudioRec),
}

impl SysAudio {
    /// Starts recording. `epoch` is the unix time of the shared recording
    /// clock. The error is the reason, ready for the UI.
    #[allow(unused_variables)]
    pub(crate) fn start(epoch: f64, out: &Path, log: &Path) -> Result<SysAudio, String> {
        let _ = std::fs::remove_file(out);
        #[cfg(windows)]
        {
            let wav = out.with_extension("wav");
            return match super::wasapi::start(super::wasapi::Source::Loopback, epoch, &wav) {
                Ok(r) => {
                    super::mic::log_line(log, &format!("system audio: WASAPI loopback of \"{}\"", r.device));
                    Ok(SysAudio { inner: Some(Inner::Wasapi(r)), out: out.to_path_buf(), log: log.to_path_buf() })
                }
                Err(e) => Err(format!("{e:#}")),
            };
        }
        #[cfg(target_os = "macos")]
        {
            let wav = out.with_extension("wav");
            return match super::macos_sck::AudioRec::start(epoch, &wav) {
                Ok(r) => {
                    super::mic::log_line(log, "system audio: ScreenCaptureKit audio stream");
                    Ok(SysAudio { inner: Some(Inner::Sck(r)), out: out.to_path_buf(), log: log.to_path_buf() })
                }
                Err(e) => Err(e),
            };
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        Err("system audio is recorded together with the screen on this system".to_string())
    }

    /// Stops and writes the sidecar. Nothing playing during the recording is
    /// fine and leaves no file; the error is for real failures.
    pub(crate) fn stop(mut self) -> Result<(), String> {
        let r: Result<(), String> = match self.inner.take() {
            #[cfg(windows)]
            Some(Inner::Wasapi(r)) => r.finish(&self.out).map_err(|e| format!("{e:#}")),
            #[cfg(target_os = "macos")]
            Some(Inner::Sck(r)) => r.finish(&self.out),
            None => Ok(()),
        };
        match r {
            Err(e) if is_silence(&e) => {
                super::mic::log_line(&self.log, "system audio: nothing was played");
                Ok(())
            }
            Err(e) => {
                super::mic::log_line(&self.log, &format!("system audio failed: {e}"));
                let _ = std::fs::remove_file(&self.out);
                Err(e)
            }
            Ok(()) => {
                super::mic::log_line(&self.log, "system audio: recorded");
                Ok(())
            }
        }
    }

    /// Stops and throws the recording away.
    pub(crate) fn discard(self) {
        let out = self.out.clone();
        let _ = self.stop();
        let _ = std::fs::remove_file(out);
    }
}

/// The WASAPI recorder reports a loopback that saw no sound as an error;
/// that only means nothing was playing.
fn is_silence(e: &str) -> bool {
    e.contains("delivered no audio")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quiet_computer_is_not_an_error() {
        assert!(is_silence("the device delivered no audio"));
        assert!(!is_silence("the device was lost"));
    }

    #[test]
    fn only_windows_and_macos_use_the_sidecar() {
        assert_eq!(supported(), cfg!(any(windows, target_os = "macos")));
    }
}
