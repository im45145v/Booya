//! Export: decode source -> render every output frame -> encode with ffmpeg.

use crate::events::EventLog;
use crate::ffmpeg::{self, FrameReader};
use crate::project::{Background, Format, Project};
use crate::render::{Frame, Renderer};
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Build the renderer for a project at its export size.
pub fn make_renderer(project: &Project, out_w: u32, out_h: u32) -> Result<Renderer> {
    let log = match &project.events {
        Some(p) if p.exists() => Some(EventLog::load(p)?),
        _ => None,
    };
    let bg = match &project.style.background {
        Background::Image { path } => ffmpeg::load_image_fit(path, out_w, out_h, project.style.bg_fit).ok().map(|mut f| {
            // Darken the photo so the recording stands out.
            let k = 1.0 - project.style.bg_dim.clamp(0.0, 0.8);
            if k < 1.0 {
                for px in f.data.chunks_exact_mut(4) {
                    for c in &mut px[..3] {
                        *c = (*c as f64 * k) as u8;
                    }
                }
            }
            f
        }),
        _ => None,
    };
    Ok(Renderer::new(project.clone(), log.as_ref(), out_w, out_h, bg))
}

/// ffmpeg filter that makes the camera frame a square of `d` pixels.
pub fn webcam_filter(w: &crate::project::Webcam, d: u32) -> String {
    format!("crop='min(iw,ih)':'min(iw,ih)',scale={d}:{d}{}", if w.mirror { ",hflip" } else { "" })
}

/// Add the webcam to a preview frame, the same way export does.
pub fn draw_preview_webcam(renderer: &Renderer, project: &Project, frame: &mut crate::render::Frame, src_t: f64) {
    let Some(w) = project.webcam.as_ref().filter(|w| w.visible && w.path.exists()) else { return };
    let d = renderer.webcam_size();
    let t = (src_t - w.offset_ms).max(0.0);
    if let Ok(c) = ffmpeg::frame_at_filtered(&w.path, t, d, d, Some(&webcam_filter(w, d))) {
        renderer.draw_webcam(frame, &c);
    }
}

/// Render a single preview frame at output time `out_ms` as PNG bytes.
/// Uses the same renderer as export.
pub fn preview_png(project: &Project, out_ms: f64, out_w: u32, out_h: u32) -> Result<Vec<u8>> {
    let r = make_renderer(project, out_w, out_h)?;
    let src_t = project.source_time(out_ms).unwrap_or(project.duration_ms);
    let (sw, sh) = r.decode_size();
    let src = ffmpeg::frame_at(&project.source, src_t, sw, sh)?;
    let mut f = r.render(&src, src_t);
    draw_preview_webcam(&r, project, &mut f, src_t);
    f.encode_png()
}

fn atempo_chain(speed: f64) -> String {
    // atempo accepts 0.5..=2.0 per instance on older ffmpeg builds.
    let mut s = speed;
    let mut parts = Vec::new();
    while s > 2.0 {
        parts.push("atempo=2.0".to_string());
        s /= 2.0;
    }
    while s < 0.5 {
        parts.push("atempo=0.5".to_string());
        s /= 0.5;
    }
    parts.push(format!("atempo={s:.5}"));
    parts.join(",")
}

/// Audio filter graph matching the clip list (trim, speed, concat), with the
/// post filters (gain, normalising, volume, fades) and no measurement.
pub fn audio_filter(project: &Project) -> String {
    audio_graph(project, 1, &post_chain(project, None, true))
}

/// The graph for input `idx`, ending in `[aout]`. `post` is appended after
/// the clips are joined (may be empty).
pub fn audio_graph(project: &Project, idx: usize, post: &str) -> String {
    let mut f = String::new();
    let n = project.clips.len();
    // A positive offset delays the audio, so it is read from earlier in the file.
    let off = project.audio.offset_ms / 1000.0;
    for (i, c) in project.clips.iter().enumerate() {
        // A short fade at each cut avoids clicks where the audio is joined.
        let len = c.out_len() / 1000.0;
        let fade = if n > 1 { (project.export.crossfade_ms / 1000.0).min(len / 3.0).max(0.0) } else { 0.0 };
        let fades = if fade > 0.005 { format!(",afade=t=in:d={fade:.3},afade=t=out:st={:.3}:d={fade:.3}", (len - fade).max(0.0)) } else { String::new() };
        let start = (c.src_start_ms / 1000.0 - off).max(0.0);
        let end = (c.src_end_ms / 1000.0 - off).max(start + 0.001);
        // A clip that starts before the (delayed) audio does gets silence first.
        let pad = ((off - c.src_start_ms / 1000.0).max(0.0) / c.speed.max(0.01) * 1000.0).round();
        let pad = if pad > 0.0 { format!(",adelay={pad:.0}:all=1") } else { String::new() };
        let rebase = if project.audio.rebase_start { "asetpts=PTS-STARTPTS," } else { "" };
        f.push_str(&format!("[{idx}:a]aresample=48000,{rebase}atrim=start={start:.3}:end={end:.3},asetpts=PTS-STARTPTS,{}{pad}{fades}[a{i}];", atempo_chain(c.speed)));
    }
    for i in 0..n {
        f.push_str(&format!("[a{i}]"));
    }
    if post.is_empty() {
        f.push_str(&format!("concat=n={n}:v=0:a=1[aout]"));
    } else {
        f.push_str(&format!("concat=n={n}:v=0:a=1[acat];[acat]{post}[aout]"));
    }
    f
}

/// The project with the source's audio lead folded in. An audio track that
/// starts later (or earlier) than the video in the file, as in recordings made
/// before 0.4.1 or with a slow microphone, would otherwise be shifted to start
/// with the picture. With `zero_based` the audio comes from a file that
/// already starts at zero (the denoised copy), so only the offset is needed.
pub fn with_audio_lead(project: &Project, zero_based: bool) -> Project {
    let mut p = project.clone();
    let v = crate::finalize::stream_times(&project.source, "v:0");
    let a = crate::finalize::stream_times(&project.source, "a:0");
    if let (Some(v), Some(a)) = (v, a) {
        let lead_ms = (a.0 - v.0) * 1000.0;
        if lead_ms.abs() > 40.0 && lead_ms.abs() < 30_000.0 {
            p.audio.offset_ms += lead_ms;
            p.audio.rebase_start = !zero_based;
        }
    }
    p
}

/// Ends an audio graph at exactly `whole_dur_s`: optional silence in front
/// (a title card), then padding behind, so a track shorter than the picture
/// never cuts the video short.
pub fn pad_audio_graph(graph: &str, front_ms: f64, whole_dur_s: f64) -> String {
    let delay = if front_ms > 0.5 { format!("adelay={front_ms:.0}:all=1,") } else { String::new() };
    format!("{}[ao0];[ao0]{delay}apad=whole_dur={whole_dur_s:.3}[aout]", graph.trim_end_matches("[aout]"))
}

/// Gain, normalising, volume and fades after the clips are joined. With
/// `normalize` false the loudness stage is skipped (used for the measuring pass).
pub fn post_chain(project: &Project, norm: Option<&crate::audio::Loudness>, normalize: bool) -> String {
    let a = &project.audio;
    let mut parts: Vec<String> = Vec::new();
    if a.gain_db.abs() > 0.05 {
        parts.push(format!("volume={:.2}dB", a.gain_db.clamp(-30.0, 40.0)));
    }
    if normalize && a.normalize {
        let n = crate::audio::normalize_chain(norm);
        if !n.is_empty() {
            parts.push(n);
        }
    }
    if (a.volume - 1.0).abs() > 0.005 {
        parts.push(format!("volume={:.3}", a.volume.clamp(0.0, 1.5)));
    }
    let out_len = project.output_duration_ms() / 1000.0;
    if a.fade_in_ms > 1.0 {
        parts.push(format!("afade=t=in:d={:.3}", (a.fade_in_ms / 1000.0).min(out_len / 2.0)));
    }
    if a.fade_out_ms > 1.0 {
        let d = (a.fade_out_ms / 1000.0).min(out_len / 2.0);
        parts.push(format!("afade=t=out:st={:.3}:d={d:.3}", (out_len - d).max(0.0)));
    }
    parts.join(",")
}

/// Measure the loudness the export audio would have before normalising.
pub fn measure_export_audio(project: &Project, audio_input: &Path) -> Option<crate::audio::Loudness> {
    let graph = audio_graph(project, 0, &post_chain(project, None, false));
    crate::audio::measure_graph(audio_input, &graph).ok().flatten()
}

pub struct ExportStats {
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub seconds: f64,
}

/// Export the project. `progress` is called with values in 0..=1.
pub fn export(project: &Project, out: &Path, mut progress: impl FnMut(f64)) -> Result<ExportStats> {
    let enc = crate::ffmpeg::best_encoder();
    match export_with(project, out, enc, &mut progress) {
        // A hardware encoder can still fail mid-run (busy, unsupported size).
        // Redo it in software rather than failing the export.
        Err(e) if enc.is_hardware() && project.export.format == Format::Mp4 && !passthrough_ok(project) => {
            eprintln!("{} failed ({e:#}); retrying with libx264", enc.name());
            progress(0.0);
            export_with(project, out, crate::ffmpeg::Encoder::X264, &mut progress)
        }
        r => r,
    }
}

/// True when the export would reproduce the source unchanged, so it can be
/// copied instead of decoded, rendered and encoded again: one full clip at
/// 1x, no zoom, no blur, no burned captions, no styling, the cursor and key
/// overlays off, and the output size equal to the source.
pub fn passthrough_ok(p: &Project) -> bool {
    let s = &p.export;
    let (ow, oh) = s.aspect.size(s.quality);
    s.format == Format::Mp4
        && p.clips.len() == 1
        && p.clips[0].speed == 1.0
        && p.clips[0].src_start_ms <= 0.0
        && p.clips[0].src_end_ms >= p.duration_ms - 1.0
        && p.zoom.is_empty()
        && p.blur.is_empty()
        && (!p.caption_style.burn_in || p.captions.is_empty())
        && p.style.padding == 0.0
        && p.style.corner_radius == 0.0
        && !p.style.glass.enabled
        && !p.cursor.visible
        && !p.keys.enabled
        && p.webcam.as_ref().is_none_or(|w| !w.visible)
        && p.card_lengths_ms() == (0.0, 0.0)
        && p.callouts.is_empty()
        && p.shapes.is_empty()
        && p.watermark.as_ref().is_none_or(|w| w.text.trim().is_empty())
        && !p.audio.denoise
        && (ow, oh) == (p.source_width, p.source_height)
}

/// Copy the streams into an MP4 without re-encoding.
fn export_passthrough(project: &Project, out: &Path, progress: &mut dyn FnMut(f64)) -> Result<ExportStats> {
    let info = ffmpeg::probe(&project.source)?;
    let mut cmd = Command::new(ffmpeg::ffmpeg_bin());
    cmd.args(["-v", "error", "-nostdin", "-y", "-i"]).arg(ffmpeg::file_url(&project.source));
    // The capture is H.264 already; audio may be PCM or Opus, which MP4
    // players handle badly, so audio is converted to AAC (cheap).
    // The picture is copied; the sound still goes through the audio graph
    // (normalising, gain, volume, fades), which is cheap.
    if project.audio.mute || !info.has_audio {
        cmd.args(["-map", "0:v:0", "-an", "-c:v", "copy"]);
    } else {
        let ap = with_audio_lead(project, false);
        let m = if ap.audio.normalize { measure_export_audio(&ap, &project.source) } else { None };
        let graph = pad_audio_graph(&audio_graph(&ap, 0, &post_chain(&ap, m.as_ref(), true)), 0.0, project.output_duration_ms() / 1000.0);
        cmd.args(["-filter_complex", &graph, "-map", "0:v:0", "-map", "[aout]", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-ar", "48000"]);
    }
    cmd.args(["-movflags", "+faststart"]);
    cmd.arg(ffmpeg::file_url(out));
    let res = cmd.stdin(Stdio::null()).output().context("failed to start ffmpeg")?;
    if !res.status.success() {
        bail!("ffmpeg copy failed: {}", String::from_utf8_lossy(&res.stderr));
    }
    progress(1.0);
    let fps = project.export.fps as f64;
    Ok(ExportStats { frames: (info.duration_ms / 1000.0 * fps) as u64, width: info.width, height: info.height, seconds: info.duration_ms / 1000.0 })
}

/// A private scratch folder (0700 on Unix) for temporary audio.
fn private_tmp() -> Result<std::path::PathBuf> {
    let d = std::env::temp_dir().join(format!("demolens-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.recursive(true).create(&d)?;
    Ok(d)
}

/// Kills the encoder if export bails out early (decoder error, panic).
struct KillOnDrop(Option<std::process::Child>);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(c) = self.0.as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

pub fn export_with(project: &Project, out: &Path, enc: crate::ffmpeg::Encoder, progress: &mut dyn FnMut(f64)) -> Result<ExportStats> {
    project.validate()?;
    if passthrough_ok(project) {
        return export_passthrough(project, out, progress);
    }
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let s = &project.export;
    let (ow, oh) = s.aspect.size(s.quality);
    let fps = s.fps.clamp(1, 60) as f64;
    let renderer = make_renderer(project, ow, oh)?;
    let info = ffmpeg::probe(&project.source)?;
    let out_dur = project.output_duration_ms();
    let total = (out_dur / 1000.0 * fps).floor() as u64;
    if total == 0 {
        bail!("nothing to export");
    }
    // Title cards sit before and after the recording.
    let (intro_ms, outro_ms) = project.card_lengths_ms();
    let intro_n = (intro_ms / 1000.0 * fps).round() as u64;
    let outro_n = (outro_ms / 1000.0 * fps).round() as u64;
    let grand = total + intro_n + outro_n;

    // Decode the source at a high enough rate that sped-up clips still have
    // a frame for every output frame.
    let max_speed = project.clips.iter().map(|c| c.speed).fold(1.0, f64::max);
    let dec_fps = (info.fps.max(fps)).min(fps * max_speed).max(fps);
    let first_src = project.clips[0].src_start_ms;
    let (dw, dh) = renderer.decode_size();
    let mut reader = FrameReader::open(&project.source, first_src, dec_fps, dw, dh)?;
    let mut current: Option<Frame> = reader.next()?;

    // Optional webcam bubble, decoded square at its on-screen size.
    let mut cam = match project.webcam.as_ref().filter(|w| w.visible && w.path.exists()) {
        Some(w) => {
            let d = renderer.webcam_size();
            let start = first_src - w.offset_ms;
            let vf = webcam_filter(w, d);
            FrameReader::open_filtered(&w.path, start, fps, &vf, d, d).ok().map(|r| (r, w.offset_ms, None::<Frame>))
        }
        None => None,
    };

    let mut cmd = Command::new(ffmpeg::ffmpeg_bin());
    cmd.args(["-v", "error", "-nostdin", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{ow}x{oh}"), "-r", &fps.to_string(), "-i", "-"]);
    let with_audio = s.format != Format::Gif && info.has_audio && !project.audio.mute;
    // Noise cleanup writes a cleaned track to a private temp folder and uses
    // it as the audio input instead of the original.
    let mut audio_input = project.source.clone();
    let tmp = if with_audio && project.audio.denoise { Some(private_tmp()?) } else { None };
    if let Some(t) = &tmp {
        let denoised = t.join("denoised.wav");
        crate::audio::denoise_file(&project.source, &denoised)?;
        audio_input = denoised;
    }
    let audio_graph_text = if with_audio {
        // Lead of the audio track over the video, folded in (see with_audio_lead).
        let ap = with_audio_lead(project, tmp.is_some());
        let m = if ap.audio.normalize { measure_export_audio(&ap, &audio_input) } else { None };
        let g = audio_graph(&ap, 1, &post_chain(&ap, m.as_ref(), true));
        // Silence under the intro, and padding to the full length so neither a
        // short microphone track nor the outro card cuts the video short.
        pad_audio_graph(&g, if intro_n > 0 { intro_ms } else { 0.0 }, (out_dur + intro_ms + outro_ms) / 1000.0)
    } else {
        String::new()
    };
    match s.format {
        Format::Mp4 => {
            if with_audio {
                cmd.arg("-i").arg(ffmpeg::file_url(&audio_input));
                cmd.args(["-filter_complex", &audio_graph_text, "-map", "0:v", "-map", "[aout]", "-c:a", "aac", "-b:a", "192k", "-ar", "48000"]);
            }
            cmd.args(enc.export_args(ow, oh, fps)).args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"]);
        }
        Format::Webm => {
            if with_audio {
                cmd.arg("-i").arg(ffmpeg::file_url(&audio_input));
                cmd.args(["-filter_complex", &audio_graph_text, "-map", "0:v", "-map", "[aout]", "-c:a", "libopus", "-b:a", "160k", "-ar", "48000"]);
            }
            cmd.args(ffmpeg::Encoder::webm_args()).args(["-pix_fmt", "yuv420p"]);
        }
        Format::Gif => {
            cmd.args(["-vf", "split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a", "-loop", "0"]);
        }
    }
    cmd.arg(ffmpeg::file_url(out));
    let mut guard = KillOnDrop(Some(cmd.stdin(Stdio::null()).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().context("failed to start ffmpeg encoder")?));
    let mut stdin = guard.0.as_mut().unwrap().stdin.take().unwrap();

    // Cross-fade at cuts: the last frame before a cut fades into the next clip.
    let fade_ms = s.crossfade_ms.max(0.0);
    let boundaries: Vec<f64> = project.clips.iter().scan(0.0, |acc, c| {
        *acc += c.out_len();
        Some(*acc)
    }).collect();
    let mut last_frame: Option<Frame> = None;
    let mut hold: Option<(f64, Frame)> = None;

    let mut written = 0u64;
    let write_card = |card: &crate::project::Card, n: u64, stdin: &mut std::process::ChildStdin, written: &mut u64, progress: &mut dyn FnMut(f64)| -> bool {
        let fade = (fps * 0.45).max(1.0);
        for i in 0..n {
            let (a, b) = (i as f64, (n - 1 - i) as f64);
            let k = (a.min(b) / fade).clamp(0.0, 1.0) as f32;
            let k = k * k * (3.0 - 2.0 * k);
            let f = renderer.card_frame(card, k);
            if stdin.write_all(&f.data).is_err() {
                return false;
            }
            *written += 1;
            progress(*written as f64 / grand as f64);
        }
        true
    };
    if let (Some(c), true) = (project.intro.as_ref(), intro_n > 0) {
        write_card(c, intro_n, &mut stdin, &mut written, progress);
    }

    for i in 0..total {
        let out_t = i as f64 * 1000.0 / fps;
        let src_t = project.source_time(out_t).unwrap_or(project.duration_ms);
        // Advance the decoder. Clips are in source order, so this only moves
        // forward; gaps between clips are skipped by decoding past them.
        let target_idx = ((src_t - first_src) / 1000.0 * dec_fps).floor().max(0.0) as u64;
        while reader.index <= target_idx {
            match reader.next()? {
                Some(f) => current = Some(f),
                None => break,
            }
        }
        let src = current.as_ref().context("source has no frames")?;
        let mut frame = renderer.render(src, src_t);
        if let Some((r, offset, cur)) = cam.as_mut() {
            let cam_t = src_t - *offset;
            let want = ((cam_t - (first_src - *offset)) / 1000.0 * r.fps).floor().max(0.0) as u64;
            while r.index <= want {
                match r.next() {
                    Ok(Some(f)) => *cur = Some(f),
                    _ => break,
                }
            }
            if let Some(c) = cur.as_ref() {
                renderer.draw_webcam(&mut frame, c);
            }
        }
        if fade_ms > 0.0 && boundaries.len() > 1 {
            // Entering a new clip: remember the previous clip's last frame.
            if let Some(&b) = boundaries.iter().find(|&&b| out_t >= b && out_t - b < 1000.0 / fps) {
                if b < out_dur - 1.0 {
                    if let Some(prev) = last_frame.take() {
                        hold = Some((b, prev));
                    }
                }
            }
            if let Some((b, prev)) = &hold {
                let k = (out_t - b) / fade_ms;
                if k >= 1.0 {
                    hold = None;
                } else {
                    frame.mix_from(prev, (1.0 - k) as f32);
                }
            }
            last_frame = Some(frame.clone());
        }
        if stdin.write_all(&frame.data).is_err() {
            break;
        }
        written += 1;
        progress(written as f64 / grand as f64);
    }
    if let (Some(c), true) = (project.outro.as_ref(), outro_n > 0) {
        write_card(c, outro_n, &mut stdin, &mut written, progress);
    }
    drop(stdin);
    let res = guard.0.take().unwrap().wait_with_output()?;
    if let Some(t) = tmp {
        let _ = std::fs::remove_dir_all(t);
    }
    if !res.status.success() {
        bail!("ffmpeg encode failed: {}", String::from_utf8_lossy(&res.stderr));
    }
    Ok(ExportStats { frames: grand, width: ow, height: oh, seconds: (out_dur + intro_ms + outro_ms) / 1000.0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atempo_chains_stay_in_range() {
        assert_eq!(atempo_chain(1.0), "atempo=1.00000");
        assert_eq!(atempo_chain(4.0), "atempo=2.0,atempo=2.00000");
        assert_eq!(atempo_chain(0.25), "atempo=0.5,atempo=0.50000");
    }

    #[test]
    fn passthrough_only_when_nothing_changes() {
        let mut p = Project::new("a.mkv".into(), None, 1920, 1080, 10_000.0);
        assert!(!passthrough_ok(&p), "default styling needs a render");
        p.style.padding = 0.0;
        p.style.corner_radius = 0.0;
        p.cursor.visible = false;
        p.keys.enabled = false;
        assert!(passthrough_ok(&p));
        p.clips[0].speed = 1.5;
        assert!(!passthrough_ok(&p));
    }

    #[test]
    fn audio_fades_only_at_cuts() {
        let mut p = Project::new("a.mkv".into(), None, 1920, 1080, 10_000.0);
        assert!(!audio_filter(&p).contains("afade"));
        p.cut_ranges(&[(4_000.0, 5_000.0)]);
        assert!(audio_filter(&p).contains("afade=t=in"));
    }

    #[test]
    fn a_quiet_recording_is_normalised_on_export() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("dl-export-loud-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("quiet.mp4");
        let ok = Command::new(ffmpeg::ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=1920x1080:r=10:d=3", "-f", "lavfi", "-i", "sine=frequency=300:d=3", "-af", "volume=-25dB", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&src)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let mut p = Project::new(src.clone(), None, 1920, 1080, 3000.0);
        p.style.padding = 0.0;
        p.style.corner_radius = 0.0;
        p.cursor.visible = false;
        p.keys.enabled = false;
        assert!(passthrough_ok(&p));
        let before = crate::audio::measure_file(&src).unwrap().unwrap().input_i;
        let out = dir.join("out.mp4");
        export(&p, &out, |_| {}).unwrap();
        let after = crate::audio::measure_file(&out).unwrap().unwrap().input_i;
        assert!(before < -40.0, "{before}");
        assert!((after - crate::audio::TARGET_LUFS).abs() < 2.5, "{after}");
        // Normalising off leaves the level alone.
        p.audio.normalize = false;
        let raw = dir.join("raw.mp4");
        export(&p, &raw, |_| {}).unwrap();
        let unchanged = crate::audio::measure_file(&raw).unwrap().unwrap().input_i;
        assert!((unchanged - before).abs() < 2.0, "{unchanged} vs {before}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn title_cards_extend_the_video_and_keep_the_sound_in_place() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("dl-export-cards-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.mp4");
        let ok = Command::new(ffmpeg::ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=1920x1080:r=10:d=3", "-f", "lavfi", "-i", "sine=frequency=300:d=3", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&src)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let mut p = Project::new(src, None, 1920, 1080, 3000.0);
        p.export.quality = 360;
        p.audio.normalize = false;
        p.intro = Some(crate::project::Card { title: "Intro".into(), subtitle: String::new(), duration_ms: 1000.0 });
        p.outro = Some(crate::project::Card { title: "Bye".into(), subtitle: "see you".into(), duration_ms: 2000.0 });
        assert!(!passthrough_ok(&p));
        let out = dir.join("out.mp4");
        export(&p, &out, |_| {}).unwrap();
        let d = crate::ffmpeg::probe(&out).unwrap().duration_ms;
        assert!((d - 6000.0).abs() < 250.0, "duration {d}");
        // The first second is silent (the intro), the sound follows.
        let quiet = Command::new(ffmpeg::ffmpeg_bin())
            .args(["-v", "error", "-i"]).arg(&out)
            .args(["-t", "0.8", "-af", "volumedetect", "-f", "null", "-"])
            .output().unwrap();
        let log = String::from_utf8_lossy(&quiet.stderr).to_string();
        let loud = Command::new(ffmpeg::ffmpeg_bin())
            .args(["-v", "info", "-ss", "1.5", "-t", "1", "-i"]).arg(&out)
            .args(["-af", "volumedetect", "-f", "null", "-"])
            .output().unwrap();
        let loud_log = String::from_utf8_lossy(&loud.stderr).to_string();
        let mean = |l: &str| l.lines().find_map(|x| x.split("mean_volume:").nth(1)).and_then(|v| v.trim().trim_end_matches(" dB").parse::<f64>().ok());
        let _ = log;
        assert!(mean(&loud_log).unwrap_or(-91.0) > -40.0, "no sound after the intro: {loud_log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn rms(x: &[f32], from_s: f64, to_s: f64, rate: f64) -> f32 {
        let (a, b) = ((from_s * rate) as usize, ((to_s * rate) as usize).min(x.len()));
        if b <= a {
            return 0.0;
        }
        (x[a..b].iter().map(|v| v * v).sum::<f32>() / (b - a) as f32).sqrt()
    }

    fn make_source(dir: &std::path::Path, name: &str, audio_args: &[&str]) -> std::path::PathBuf {
        let src = dir.join(name);
        let mut args = vec!["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=1920x1080:r=10:d=3"];
        args.extend_from_slice(audio_args);
        args.extend(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]);
        assert!(Command::new(ffmpeg::ffmpeg_bin()).args(&args).arg(&src).status().unwrap().success());
        src
    }

    #[test]
    fn a_short_audio_track_does_not_cut_the_video_short() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("dl-export-short-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 3 s of picture, only 1.5 s of sound.
        let src = make_source(&dir, "a.mkv", &["-f", "lavfi", "-i", "sine=frequency=300:d=1.5"]);
        let mut p = Project::new(src, None, 1920, 1080, 3000.0);
        p.export.quality = 360;
        p.audio.normalize = false;
        let out = dir.join("out.mp4");
        export(&p, &out, |_| {}).unwrap();
        let d = crate::ffmpeg::probe(&out).unwrap().duration_ms;
        assert!((d - 3000.0).abs() < 250.0, "the export was cut to {d} ms");
        let x = crate::audio::decode_mono(&out, 8000).unwrap();
        assert!((x.len() as f64 / 8000.0 - 3.0).abs() < 0.3, "audio is {} s", x.len() as f64 / 8000.0);
        assert!(rms(&x, 0.2, 1.2, 8000.0) > 0.05 && rms(&x, 2.0, 2.8, 8000.0) < 0.001);
        let rate = crate::finalize::audio_rate(&out);
        assert_eq!(rate, Some(48_000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audio_that_starts_late_in_the_file_stays_late_in_the_export() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("dl-export-late-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The sound starts 1.0 s after the picture, as in a recording whose
        // microphone took a moment to open.
        let src = make_source(&dir, "late.mkv", &["-itsoffset", "1.0", "-f", "lavfi", "-i", "sine=frequency=300:d=2"]);
        let mut p = Project::new(src, None, 1920, 1080, 3000.0);
        p.export.quality = 360;
        p.audio.normalize = false;
        let out = dir.join("out.mp4");
        export(&p, &out, |_| {}).unwrap();
        let x = crate::audio::decode_mono(&out, 8000).unwrap();
        assert!(rms(&x, 0.1, 0.8, 8000.0) < 0.001, "the sound moved to the start of the video");
        assert!(rms(&x, 1.3, 2.7, 8000.0) > 0.05, "no sound where it was recorded");
        let d = crate::ffmpeg::probe(&out).unwrap().duration_ms;
        assert!((d - 3000.0).abs() < 250.0, "duration {d}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audio_offset_and_fades_reach_the_graph() {
        let mut p = Project::new("a.mkv".into(), None, 1920, 1080, 10_000.0);
        p.audio.offset_ms = 300.0;
        p.audio.volume = 0.5;
        p.audio.fade_out_ms = 500.0;
        let g = audio_filter(&p);
        assert!(g.contains("adelay=300"), "{g}");
        assert!(g.contains("volume=0.500") && g.contains("afade=t=out"), "{g}");
        p.audio.offset_ms = -200.0;
        // Earlier audio reads later in the file.
        assert!(audio_filter(&p).contains("atrim=start=0.200"));
    }
}
