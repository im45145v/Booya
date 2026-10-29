//! Native playback of the editor's sound.
//!
//! The editor used to play its sound through an HTML audio element. In the
//! macOS app's web view that element can load a file, report no error and
//! still play nothing. So the sound is played here instead, straight to the
//! system's output device, and the web page only tells this module where the
//! playhead is. The picture stays in the (muted) video element.
//!
//! * [`Mixer`] holds the tracks (the recording's voice and an optional music
//!   track), the transport (position, speed, volume) and turns them into
//!   output samples. It is plain code with no audio device, so it is tested
//!   everywhere.
//! * [`Player`] (Windows and macOS) opens the default output device with cpal
//!   and runs a `Mixer` in its callback.
//! * [`decode`] reads any audio file into 16 bit PCM through ffmpeg.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};

/// Decoded audio, interleaved 16 bit.
#[derive(Debug, Clone, Default)]
pub struct Pcm {
    pub data: Vec<i16>,
    pub channels: usize,
    pub rate: u32,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.data.len() / self.channels
        }
    }
    pub fn seconds(&self) -> f64 {
        if self.rate == 0 {
            0.0
        } else {
            self.frames() as f64 / self.rate as f64
        }
    }
    /// Sample of channel `ch` at fractional frame `pos`, linearly interpolated.
    /// Beyond the end it is silence.
    fn at(&self, pos: f64, ch: usize) -> f32 {
        if pos < 0.0 {
            return 0.0;
        }
        let i = pos.floor() as usize;
        let n = self.frames();
        if i >= n {
            return 0.0;
        }
        let c = ch.min(self.channels - 1);
        let a = self.data[i * self.channels + c] as f32;
        let b = if i + 1 < n { self.data[(i + 1) * self.channels + c] as f32 } else { a };
        let f = (pos - i as f64) as f32;
        (a + (b - a) * f) / 32768.0
    }
}

/// Decodes `path` (any format ffmpeg reads) to 16 bit PCM at `rate` Hz with
/// `channels` channels. `max_secs` keeps a huge file from filling memory.
pub fn decode(path: &Path, channels: usize, rate: u32, max_secs: f64) -> Result<Pcm> {
    let out = crate::proc::command(crate::ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(crate::ffmpeg::file_url(path))
        .args(["-vn", "-t", &format!("{max_secs:.1}"), "-ac", &channels.to_string(), "-ar", &rate.to_string(), "-f", "s16le", "-"])
        .stdin(std::process::Stdio::null())
        .output()
        .context("could not start ffmpeg to read the sound")?;
    if !out.status.success() {
        bail!("could not read the sound file: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
    }
    let data: Vec<i16> = out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    if data.is_empty() {
        bail!("the sound file has no audio in it");
    }
    Ok(Pcm { data, channels, rate })
}

/// How the music behaves in the preview. It mirrors [`crate::project::Music`]
/// and the export's mix, so what you hear while editing is what you export.
#[derive(Debug, Clone, PartialEq)]
pub struct MusicCfg {
    pub volume: f32,
    pub duck: bool,
    pub duck_db: f32,
    pub fade_in_s: f64,
    pub fade_out_s: f64,
    pub start_s: f64,
    pub looped: bool,
    /// Length of the whole finished video, so the fade out lands at its end.
    pub total_s: f64,
}

impl MusicCfg {
    pub fn from_project(m: &crate::project::Music, total_s: f64) -> Self {
        let m = m.clamped();
        Self { volume: m.volume as f32, duck: m.duck, duck_db: m.duck_db as f32, fade_in_s: m.fade_in_ms / 1000.0, fade_out_s: m.fade_out_ms / 1000.0, start_s: m.start_ms / 1000.0, looped: m.looped, total_s }
    }
}

/// What the player reports to the page and to the diagnostics readout.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    pub playing: bool,
    /// Where the voice track is, in seconds of the recording.
    pub voice_s: f64,
    /// Where the music is, in seconds of the finished video.
    pub music_s: f64,
    pub rate: f64,
    pub volume: f32,
    pub muted: bool,
    /// Loudest output sample since the last call (0 to 1). Zero while playing
    /// means nothing is coming out.
    pub peak: f32,
    pub has_voice: bool,
    pub has_music: bool,
}

pub struct Mixer {
    voice: Option<Pcm>,
    music: Option<(Pcm, MusicCfg)>,
    out_rate: u32,
    out_channels: usize,
    playing: bool,
    voice_pos: f64,
    music_pos: f64,
    rate: f64,
    volume: f32,
    muted: bool,
    env: f32,
    peak: f32,
}

/// The voice level above which music is pushed down.
const DUCK_FULL_AT: f32 = 0.05;

impl Mixer {
    pub fn new(out_rate: u32, out_channels: usize) -> Self {
        Self { voice: None, music: None, out_rate: out_rate.max(8000), out_channels: out_channels.max(1), playing: false, voice_pos: 0.0, music_pos: 0.0, rate: 1.0, volume: 1.0, muted: false, env: 0.0, peak: 0.0 }
    }

    pub fn set_voice(&mut self, pcm: Option<Pcm>) {
        self.voice = pcm;
        self.voice_pos = 0.0;
        self.env = 0.0;
    }
    pub fn set_music(&mut self, m: Option<(Pcm, MusicCfg)>) {
        self.music = m;
    }
    pub fn play(&mut self) {
        self.playing = true;
    }
    pub fn pause(&mut self) {
        self.playing = false;
    }
    /// Move the voice to `voice_s` and the music to `music_s` (seconds into the finished video).
    pub fn seek(&mut self, voice_s: f64, music_s: f64) {
        self.voice_pos = voice_s.max(0.0);
        self.music_pos = music_s.max(0.0);
        self.env = 0.0;
    }
    /// Speed of the voice (the clip's speed). The music follows the finished video, so it always runs at 1x.
    pub fn set_rate(&mut self, r: f64) {
        self.rate = r.clamp(0.25, 4.0);
    }
    pub fn set_volume(&mut self, v: f32, muted: bool) {
        self.volume = v.clamp(0.0, 1.0);
        self.muted = muted;
    }
    pub fn state(&mut self) -> State {
        let s = State { playing: self.playing, voice_s: self.voice_pos, music_s: self.music_pos, rate: self.rate, volume: self.volume, muted: self.muted, peak: self.peak, has_voice: self.voice.is_some(), has_music: self.music.is_some() };
        self.peak = 0.0;
        s
    }

    /// The music's gain at `t` seconds into the finished video, without the ducking.
    pub fn music_gain(cfg: &MusicCfg, t: f64) -> f32 {
        let local = t - cfg.start_s;
        if local < 0.0 {
            return 0.0;
        }
        let len = (cfg.total_s - cfg.start_s).max(0.1);
        let fi = cfg.fade_in_s.min(len / 2.0);
        let fo = cfg.fade_out_s.min(len / 2.0);
        let mut g = cfg.volume;
        if fi > 0.01 && local < fi {
            g *= (local / fi) as f32;
        }
        if fo > 0.01 && local > len - fo {
            g *= (((len - local) / fo).max(0.0)) as f32;
        }
        g
    }

    /// Fill `out` (interleaved, `out_channels` wide) with the next samples.
    pub fn render(&mut self, out: &mut [f32]) {
        let ch = self.out_channels;
        if !self.playing {
            out.fill(0.0);
            return;
        }
        let step_voice = self.rate / self.out_rate as f64;
        let dt = 1.0 / self.out_rate as f64;
        // Envelope follower for the ducking: quick to catch speech, slow to let go.
        let attack = 1.0 - (-1.0 / (0.03 * self.out_rate as f32)).exp();
        let release = 1.0 - (-1.0 / (0.25 * self.out_rate as f32)).exp();
        let master = if self.muted { 0.0 } else { self.volume };
        let mut peak = self.peak;
        for frame in out.chunks_mut(ch) {
            let mut v = 0.0f32;
            if let Some(p) = &self.voice {
                v = p.at(self.voice_pos * p.rate as f64, 0);
            }
            let a = v.abs();
            self.env += (a - self.env) * if a > self.env { attack } else { release };
            let (mut ml, mut mr) = (0.0f32, 0.0f32);
            if let Some((p, cfg)) = &self.music {
                let local = self.music_pos - cfg.start_s;
                if local >= 0.0 {
                    let dur = p.seconds();
                    let t = if cfg.looped && dur > 0.0 { local % dur } else { local };
                    if t < dur {
                        let pos = t * p.rate as f64;
                        let mut g = Self::music_gain(cfg, self.music_pos);
                        if cfg.duck {
                            let amount = (self.env / DUCK_FULL_AT).clamp(0.0, 1.0);
                            g *= 1.0 - amount * (1.0 - 10f32.powf(-cfg.duck_db / 20.0));
                        }
                        ml = p.at(pos, 0) * g;
                        mr = p.at(pos, 1) * g;
                    }
                }
            }
            match ch {
                1 => frame[0] = ((v + (ml + mr) * 0.5) * master).clamp(-1.0, 1.0),
                _ => {
                    frame[0] = ((v + ml) * master).clamp(-1.0, 1.0);
                    frame[1] = ((v + mr) * master).clamp(-1.0, 1.0);
                    for s in frame.iter_mut().skip(2) {
                        *s = 0.0;
                    }
                }
            }
            peak = peak.max(frame.iter().fold(0.0f32, |m, s| m.max(s.abs())));
            self.voice_pos += step_voice;
            self.music_pos += dt;
        }
        self.peak = peak;
    }
}

/// True when the sound has drifted from where the video says it should be
/// by more than `tol_s`, so the page should seek it.
pub fn needs_reseek(want_s: f64, have_s: f64, tol_s: f64) -> bool {
    (want_s - have_s).abs() > tol_s
}

pub type SharedMixer = Arc<Mutex<Mixer>>;

#[cfg(any(windows, target_os = "macos"))]
pub use output::Player;

#[cfg(any(windows, target_os = "macos"))]
mod output {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    /// The default output device with a mixer running in its callback. The
    /// stream lives on its own thread (some systems' streams cannot move
    /// between threads), which ends when the player is dropped.
    pub struct Player {
        pub mixer: SharedMixer,
        pub device: String,
        pub rate: u32,
        pub channels: usize,
        stop: Arc<AtomicBool>,
        err: Arc<Mutex<Option<String>>>,
        join: Option<std::thread::JoinHandle<()>>,
    }

    impl Player {
        pub fn open() -> Result<Self> {
            let stop = Arc::new(AtomicBool::new(false));
            let err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
            let (tx, rx) = std::sync::mpsc::channel::<Result<(SharedMixer, String, u32, usize), String>>();
            let (stop2, err2) = (stop.clone(), err.clone());
            let join = std::thread::Builder::new()
                .name("booya-playback".into())
                .spawn(move || {
                    let setup = || -> Result<(cpal::Stream, SharedMixer, String, u32, usize)> {
                        let host = cpal::default_host();
                        let dev = host.default_output_device().ok_or_else(|| anyhow::anyhow!("this computer has no sound output"))?;
                        let conf = dev.default_output_config().map_err(|e| anyhow::anyhow!("{e}"))?;
                        let (rate, channels) = (conf.sample_rate(), conf.channels() as usize);
                        let name = dev.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "default output".into());
                        let mixer: SharedMixer = Arc::new(Mutex::new(Mixer::new(rate, channels)));
                        let cfg = conf.config();
                        let stream = match conf.sample_format() {
                            cpal::SampleFormat::F32 => build::<f32>(&dev, cfg, mixer.clone(), err2.clone())?,
                            cpal::SampleFormat::I16 => build::<i16>(&dev, cfg, mixer.clone(), err2.clone())?,
                            cpal::SampleFormat::I32 => build::<i32>(&dev, cfg, mixer.clone(), err2.clone())?,
                            other => bail!("the sound output uses a sample format ({other:?}) Booya cannot write"),
                        };
                        stream.play().map_err(|e| anyhow::anyhow!("{e}"))?;
                        Ok((stream, mixer, name, rate, channels))
                    };
                    match setup() {
                        Ok((stream, mixer, name, rate, channels)) => {
                            let _ = tx.send(Ok((mixer, name, rate, channels)));
                            while !stop2.load(Ordering::SeqCst) {
                                std::thread::sleep(Duration::from_millis(50));
                            }
                            drop(stream);
                        }
                        Err(e) => {
                            let _ = tx.send(Err(format!("{e:#}")));
                        }
                    }
                })
                .context("could not start the playback thread")?;
            match rx.recv_timeout(Duration::from_secs(8)) {
                Ok(Ok((mixer, device, rate, channels))) => Ok(Self { mixer, device, rate, channels, stop, err, join: Some(join) }),
                Ok(Err(e)) => bail!("{e}"),
                Err(_) => bail!("the sound output did not answer in time"),
            }
        }

        /// The last error the output device reported, if any.
        pub fn last_error(&self) -> Option<String> {
            self.err.lock().ok().and_then(|e| e.clone())
        }
    }

    impl Drop for Player {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(j) = self.join.take() {
                let _ = j.join();
            }
        }
    }

    fn build<T>(dev: &cpal::Device, cfg: cpal::StreamConfig, mixer: SharedMixer, err: Arc<Mutex<Option<String>>>) -> Result<cpal::Stream>
    where
        T: cpal::SizedSample + cpal::FromSample<f32> + Send + 'static,
    {
        let mut buf: Vec<f32> = Vec::new();
        dev.build_output_stream::<T, _, _>(
            cfg,
            move |data: &mut [T], _| {
                buf.resize(data.len(), 0.0);
                match mixer.try_lock() {
                    Ok(mut m) => m.render(&mut buf),
                    // A command holds the mixer for a moment: a few samples of silence.
                    Err(_) => buf.fill(0.0),
                }
                for (o, s) in data.iter_mut().zip(buf.iter()) {
                    *o = T::from_sample_(*s);
                }
            },
            move |e| {
                if let Ok(mut g) = err.lock() {
                    *g = Some(e.to_string());
                }
            },
            Some(Duration::from_secs(5)),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, secs: f64, rate: u32, amp: f64, channels: usize) -> Pcm {
        let n = (secs * rate as f64) as usize;
        let mut data = Vec::with_capacity(n * channels);
        for i in 0..n {
            let v = ((i as f64 / rate as f64 * hz * std::f64::consts::TAU).sin() * amp * 32767.0) as i16;
            for _ in 0..channels {
                data.push(v);
            }
        }
        Pcm { data, channels, rate }
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// Renders `secs` of output and returns the left channel.
    fn run(m: &mut Mixer, secs: f64) -> Vec<f32> {
        let mut out = vec![0.0; (secs * m.out_rate as f64) as usize * m.out_channels];
        m.render(&mut out);
        out.chunks(m.out_channels).map(|f| f[0]).collect()
    }

    #[test]
    fn paused_is_silent_and_playing_makes_sound_and_moves_the_clock() {
        let mut m = Mixer::new(48_000, 2);
        m.set_voice(Some(tone(440.0, 2.0, 48_000, 0.5, 1)));
        assert!(rms(&run(&mut m, 0.2)) == 0.0, "paused must be silent");
        assert_eq!(m.state().voice_s, 0.0);
        m.play();
        let x = run(&mut m, 0.5);
        assert!((rms(&x) - 0.354).abs() < 0.03, "{}", rms(&x));
        let s = m.state();
        assert!((s.voice_s - 0.5).abs() < 0.001, "{}", s.voice_s);
        assert!(s.peak > 0.4 && s.playing, "{s:?}");
        // The peak resets once read.
        assert_eq!(m.state().peak, 0.0);
        m.pause();
        assert_eq!(rms(&run(&mut m, 0.1)), 0.0);
    }

    #[test]
    fn seek_rate_and_the_end_of_the_track() {
        let mut m = Mixer::new(48_000, 2);
        // First half a second is loud, then silence.
        let mut p = tone(440.0, 0.5, 48_000, 0.5, 1);
        p.data.extend(std::iter::repeat_n(0i16, 48_000));
        m.set_voice(Some(p));
        m.play();
        m.seek(0.9, 0.0);
        assert_eq!(rms(&run(&mut m, 0.1)), 0.0);
        m.seek(0.0, 0.0);
        assert!(rms(&run(&mut m, 0.1)) > 0.2);
        // At 2x the clock runs twice as fast.
        m.seek(0.0, 0.0);
        m.set_rate(2.0);
        run(&mut m, 0.25);
        assert!((m.state().voice_s - 0.5).abs() < 0.001);
        // Past the end of the file: silence, and no crash.
        m.seek(100.0, 0.0);
        assert_eq!(rms(&run(&mut m, 0.1)), 0.0);
        // Rate is kept in range.
        m.set_rate(100.0);
        assert_eq!(m.state().rate, 4.0);
    }

    #[test]
    fn a_different_rate_between_file_and_device_plays_at_the_right_speed() {
        // A 24 kHz file on a 48 kHz device: a 1 s tone is still 1 s.
        let mut m = Mixer::new(48_000, 2);
        m.set_voice(Some(tone(300.0, 1.0, 24_000, 0.5, 1)));
        m.play();
        run(&mut m, 0.5);
        assert!((m.state().voice_s - 0.5).abs() < 0.001);
        // Position is in seconds of the recording, so the content reached is at 0.5 s of the tone.
        let x = run(&mut m, 0.6);
        assert!(rms(&x[..24_000]) > 0.2 && rms(&x[28_000..]) == 0.0, "the tone should end at 1 s");
    }

    #[test]
    fn volume_and_mute_apply_to_the_output() {
        let mut m = Mixer::new(44_100, 2);
        m.set_voice(Some(tone(300.0, 2.0, 44_100, 0.8, 1)));
        m.play();
        let full = rms(&run(&mut m, 0.2));
        m.set_volume(0.5, false);
        let half = rms(&run(&mut m, 0.2));
        assert!((half / full - 0.5).abs() < 0.05, "{half} vs {full}");
        m.set_volume(1.0, true);
        assert_eq!(rms(&run(&mut m, 0.2)), 0.0);
        // Sound goes to every output channel, and extra channels stay silent.
        let mut m6 = Mixer::new(44_100, 6);
        m6.set_voice(Some(tone(300.0, 1.0, 44_100, 0.8, 1)));
        m6.play();
        let mut out = vec![0.0; 600];
        m6.render(&mut out);
        assert!(out.chunks(6).skip(10).any(|f| f[0].abs() > 0.1 && f[1].abs() > 0.1 && f[2..].iter().all(|s| *s == 0.0)));
    }

    #[test]
    fn music_starts_loops_and_fades() {
        let mut m = Mixer::new(48_000, 2);
        let cfg = MusicCfg { volume: 1.0, duck: false, duck_db: 12.0, fade_in_s: 1.0, fade_out_s: 1.0, start_s: 1.0, looped: true, total_s: 5.0 };
        // A 1.5 s tone that must loop to fill the video.
        m.set_music(Some((tone(500.0, 1.5, 48_000, 0.5, 2), cfg)));
        m.play();
        let x = run(&mut m, 5.0);
        let at = |a: f64, b: f64| rms(&x[(a * 48_000.0) as usize..(b * 48_000.0) as usize]);
        assert_eq!(at(0.0, 0.9), 0.0, "nothing before the start");
        assert!(at(1.0, 1.3) < at(1.9, 2.1) / 2.0, "fades in");
        assert!(at(1.9, 2.1) > 0.25);
        assert!(at(3.0, 3.5) > 0.25, "looped past its 1.5 s");
        assert!(at(4.8, 5.0) < at(2.9, 3.1) / 3.0, "fades out at the end of the video");
    }

    #[test]
    fn music_ducks_under_the_voice_and_comes_back() {
        let mk = |duck: bool| {
            let mut m = Mixer::new(48_000, 2);
            // Voice for the first second, then silence.
            let mut v = tone(300.0, 1.0, 48_000, 0.5, 1);
            v.data.extend(std::iter::repeat_n(0i16, 96_000));
            m.set_voice(Some(v));
            m.set_music(Some((tone(900.0, 3.0, 48_000, 0.6, 2), MusicCfg { volume: 1.0, duck, duck_db: 18.0, fade_in_s: 0.0, fade_out_s: 0.0, start_s: 0.0, looped: true, total_s: 3.0 })));
            m.play();
            run(&mut m, 3.0)
        };
        let (d, n) = (mk(true), mk(false));
        let r = |x: &[f32], a: f64, b: f64| rms(&x[(a * 48_000.0) as usize..(b * 48_000.0) as usize]);
        // Under the voice the mix is quieter than without ducking.
        assert!(r(&d, 0.4, 0.9) < r(&n, 0.4, 0.9) * 0.9, "{} vs {}", r(&d, 0.4, 0.9), r(&n, 0.4, 0.9));
        // After the release the music is back at full level.
        assert!((r(&d, 2.0, 2.9) - r(&n, 2.0, 2.9)).abs() < 0.02, "{} vs {}", r(&d, 2.0, 2.9), r(&n, 2.0, 2.9));
    }

    #[test]
    fn drift_is_corrected_beyond_sixty_milliseconds_only() {
        assert!(!needs_reseek(10.0, 10.05, 0.06));
        assert!(!needs_reseek(10.0, 9.95, 0.06));
        assert!(needs_reseek(10.0, 10.07, 0.06));
        assert!(needs_reseek(10.0, 9.9, 0.06));
    }

    #[test]
    fn decode_reads_a_real_file_to_pcm() {
        if !crate::ffmpeg::ffmpeg_found() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("dl-playback-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("t.m4a");
        assert!(crate::proc::command(crate::ffmpeg::ffmpeg_bin()).args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:d=1.0", "-c:a", "aac"]).arg(&f).status().unwrap().success());
        let p = decode(&f, 1, 48_000, 60.0).unwrap();
        assert!((p.seconds() - 1.0).abs() < 0.1, "{}", p.seconds());
        let st = decode(&f, 2, 44_100, 60.0).unwrap();
        assert_eq!((st.channels, st.rate), (2, 44_100));
        // It plays through the mixer.
        let mut m = Mixer::new(48_000, 2);
        m.set_voice(Some(p));
        m.play();
        assert!(rms(&run(&mut m, 0.3)) > 0.03);
        // Errors are readable.
        let e = decode(&dir.join("missing.mp3"), 1, 48_000, 10.0).unwrap_err().to_string();
        assert!(e.contains("could not read the sound file"), "{e}");
        // A cap on length.
        assert!(decode(&f, 1, 48_000, 0.3).unwrap().seconds() < 0.5);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
