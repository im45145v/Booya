//! In-process audio capture on Windows through WASAPI (cpal). The microphone
//! is opened by the same names Windows shows, with no device name passed to
//! another program, so quoting, encodings and "friendly name versus device id"
//! cannot go wrong, and the samples are checked live so a silent device is
//! noticed within a second or two. System audio is the same call on the
//! default output device (loopback).

use super::audiosink::{GapFiller, Probe, WavWriter};
use anyhow::{anyhow, bail, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum Source {
    /// A microphone by name (None = the Windows default).
    Mic(Option<String>),
    /// Everything the default speakers play.
    Loopback,
}

/// Names of the input devices, the default first.
pub(crate) fn input_names() -> Vec<String> {
    let host = cpal::default_host();
    let def = host.default_input_device().and_then(|d| name_of(&d));
    let mut v: Vec<String> = host.input_devices().map(|it| it.filter_map(|d| name_of(&d)).collect()).unwrap_or_default();
    v.dedup();
    if let Some(d) = def {
        v.retain(|n| *n != d);
        v.insert(0, d);
    }
    v
}

fn name_of(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|x| x.name().to_string())
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

struct Finished {
    frames: u64,
    rate: u32,
    channels: u16,
    /// When the first sample was taken, in seconds after the recording clock started.
    offset_s: f64,
}

pub(crate) struct Recorder {
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
    pub probe: Arc<Probe>,
    err: Arc<Mutex<Option<String>>>,
    result: Arc<Mutex<Option<Finished>>>,
    pub wav: PathBuf,
    pub device: String,
    started: Instant,
}

struct Shared {
    writer: Mutex<Option<WavWriter<BufWriter<std::fs::File>>>>,
    filler: Mutex<GapFiller>,
    probe: Arc<Probe>,
    epoch: f64,
    channels: u16,
    err: Arc<Mutex<Option<String>>>,
    /// Level meter: called about every 100 ms with (rms dB, peak dB).
    level: Option<Box<dyn Fn(f32, f32) + Send + Sync>>,
    acc: Mutex<(f64, f32, u64, Instant)>,
}

impl Shared {
    fn push(&self, samples: &[f32]) {
        let ch = self.channels.max(1) as usize;
        let frames = samples.len() / ch;
        self.probe.feed(samples, frames);
        if let Some(cb) = &self.level {
            let mut a = self.acc.lock().unwrap();
            for s in samples {
                a.0 += (*s as f64) * (*s as f64);
                a.1 = a.1.max(s.abs());
            }
            a.2 += samples.len() as u64;
            if a.3.elapsed() >= Duration::from_millis(100) && a.2 > 0 {
                let db = |v: f64| if v <= 1e-5 { -90.0 } else { (20.0 * v.log10()) as f32 };
                cb(db((a.0 / a.2 as f64).sqrt()), db(a.1 as f64));
                *a = (0.0, 0.0, 0, Instant::now());
            }
        }
        let now_ms = (unix_now() - self.epoch) * 1000.0;
        let pad = self.filler.lock().unwrap().before(now_ms, frames);
        if let Some(w) = self.writer.lock().unwrap().as_mut() {
            let r = (|| {
                if pad > 0 {
                    w.write_silence(pad)?;
                }
                w.write(samples)
            })();
            if let Err(e) = r {
                *self.err.lock().unwrap() = Some(format!("could not write the audio file: {e}"));
            }
        }
    }
}

fn build<T>(dev: &cpal::Device, cfg: cpal::StreamConfig, sh: Arc<Shared>) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    let err = sh.err.clone();
    let mut buf: Vec<f32> = Vec::new();
    dev.build_input_stream::<T, _, _>(
        cfg,
        move |data: &[T], _| {
            buf.clear();
            buf.extend(data.iter().map(|s| <f32 as cpal::FromSample<T>>::from_sample_(*s)));
            sh.push(&buf);
        },
        move |e| {
            *err.lock().unwrap() = Some(e.to_string());
        },
        Some(Duration::from_secs(5)),
    )
    .map_err(|e| anyhow!("{e}"))
}

/// Starts recording `source` to `wav` (32 bit float). `epoch` is the unix time
/// the recording clock started (the same one the screen's ffmpeg uses).
pub(crate) fn start(source: Source, epoch: f64, wav: &Path) -> Result<Recorder> {
    start_with(source, epoch, Some(wav), None)
}

/// A level meter on a microphone: nothing is written.
pub(crate) fn meter(device: Option<String>, on_level: impl Fn(f32, f32) + Send + Sync + 'static) -> Result<Recorder> {
    start_with(Source::Mic(device), unix_now(), None, Some(Box::new(on_level)))
}

fn start_with(source: Source, epoch: f64, wav: Option<&Path>, level: Option<Box<dyn Fn(f32, f32) + Send + Sync>>) -> Result<Recorder> {
    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    let stop = Arc::new(AtomicBool::new(false));
    let probe = Arc::new(Probe::default());
    let err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let result: Arc<Mutex<Option<Finished>>> = Arc::new(Mutex::new(None));
    let (stop2, probe2, err2, result2, wav2) = (stop.clone(), probe.clone(), err.clone(), result.clone(), wav.map(Path::to_path_buf));
    let mut level = level;
    let join = std::thread::Builder::new()
        .name("demolens-wasapi".into())
        .spawn(move || {
            // Everything that holds the device lives on this thread.
            let mut setup = || -> Result<(cpal::Stream, Arc<Shared>, String, u32)> {
                let host = cpal::default_host();
                let (dev, conf) = match &source {
                    Source::Mic(want) => {
                        let names = input_names();
                        let def = names.first().cloned();
                        let pick = super::mic::pick_device_name(&names, want.as_deref(), def.as_deref()).ok_or_else(|| anyhow!("Windows lists no microphone (is one plugged in and enabled?)"))?;
                        let dev = host
                            .input_devices()
                            .map_err(|e| anyhow!("{e}"))?
                            .find(|d| name_of(d).as_deref() == Some(pick.as_str()))
                            .or_else(|| host.default_input_device())
                            .ok_or_else(|| anyhow!("the microphone '{pick}' could not be opened"))?;
                        let conf = dev.default_input_config().map_err(|e| anyhow!("{e}"))?;
                        (dev, conf)
                    }
                    Source::Loopback => {
                        let dev = host.default_output_device().ok_or_else(|| anyhow!("Windows has no default speakers to listen to"))?;
                        let conf = dev.default_output_config().map_err(|e| anyhow!("{e}"))?;
                        (dev, conf)
                    }
                };
                let name = name_of(&dev).unwrap_or_else(|| "audio device".into());
                let rate = conf.sample_rate();
                let channels = conf.channels();
                let writer = match &wav2 {
                    Some(p) => Some(WavWriter::new(BufWriter::new(std::fs::File::create(p)?), rate, channels)?),
                    None => None,
                };
                let sh = Arc::new(Shared { writer: Mutex::new(writer), filler: Mutex::new(GapFiller::new(rate)), probe: probe2.clone(), epoch, channels, err: err2.clone(), level: level.take(), acc: Mutex::new((0.0, 0.0, 0, Instant::now())) });
                let cfg = conf.config();
                let stream = match conf.sample_format() {
                    cpal::SampleFormat::F32 => build::<f32>(&dev, cfg, sh.clone())?,
                    cpal::SampleFormat::I16 => build::<i16>(&dev, cfg, sh.clone())?,
                    cpal::SampleFormat::U16 => build::<u16>(&dev, cfg, sh.clone())?,
                    cpal::SampleFormat::I32 => build::<i32>(&dev, cfg, sh.clone())?,
                    other => bail!("the device uses a sample format ({other:?}) Booya cannot read"),
                };
                stream.play().map_err(|e| anyhow!("{e}"))?;
                Ok((stream, sh, name, rate))
            };
            match setup() {
                Ok((stream, sh, name, rate)) => {
                    let _ = tx.send(Ok(name));
                    while !stop2.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    drop(stream);
                    let w = sh.writer.lock().unwrap().take();
                    let first = sh.filler.lock().unwrap().first_ms();
                    if let Some(w) = w {
                        let channels = sh.channels;
                        match w.finish() {
                            Ok(frames) => *result2.lock().unwrap() = Some(Finished { frames, rate, channels, offset_s: first.unwrap_or(0.0).max(0.0) / 1000.0 }),
                            Err(e) => *err2.lock().unwrap() = Some(format!("could not finish the audio file: {e}")),
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("{e:#}")));
                }
            }
        })?;
    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(Ok(device)) => Ok(Recorder { stop, join: Some(join), probe, err, result, wav: wav.map(Path::to_path_buf).unwrap_or_default(), device, started: Instant::now() }),
        Ok(Err(e)) => {
            let _ = join.join();
            Err(anyhow!("{e}"))
        }
        Err(_) => {
            stop.store(true, Ordering::SeqCst);
            Err(anyhow!("Windows did not open the audio device within 8 seconds"))
        }
    }
}

impl Recorder {
    pub fn error(&self) -> Option<String> {
        self.err.lock().unwrap().clone()
    }

    pub fn age(&self) -> Duration {
        self.started.elapsed()
    }

    /// Stops, then writes `out` (Matroska, float samples stamped where the
    /// first sample was taken on the recording clock) from the WAV.
    pub fn finish(mut self, out: &Path) -> Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        let r = self.result.lock().unwrap().take();
        let Some(f) = r else { bail!("{}", self.error().unwrap_or_else(|| "no audio was written".into())) };
        if f.frames == 0 {
            bail!("the device delivered no audio");
        }
        let _ = (f.rate, f.channels);
        super::audiosink::wav_to_mkv(&self.wav, out, f.offset_s)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
