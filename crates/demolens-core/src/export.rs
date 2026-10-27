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
        Background::Image { path } => ffmpeg::load_image_cover(path, out_w, out_h).ok(),
        _ => None,
    };
    Ok(Renderer::new(project.clone(), log.as_ref(), out_w, out_h, bg))
}

/// Render a single preview frame at output time `out_ms` as PNG bytes.
/// Uses the same renderer as export.
pub fn preview_png(project: &Project, out_ms: f64, out_w: u32, out_h: u32) -> Result<Vec<u8>> {
    let r = make_renderer(project, out_w, out_h)?;
    let src_t = project.source_time(out_ms).unwrap_or(project.duration_ms);
    let (sw, sh) = r.decode_size();
    let src = ffmpeg::frame_at(&project.source, src_t, sw, sh)?;
    r.render(&src, src_t).encode_png()
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

/// Audio filter graph matching the clip list (trim, speed, concat).
pub fn audio_filter(project: &Project) -> String {
    let mut f = String::new();
    let n = project.clips.len();
    for (i, c) in project.clips.iter().enumerate() {
        f.push_str(&format!(
            "[1:a]atrim=start={:.3}:end={:.3},asetpts=PTS-STARTPTS,{}[a{i}];",
            c.src_start_ms / 1000.0,
            c.src_end_ms / 1000.0,
            atempo_chain(c.speed)
        ));
    }
    for i in 0..n {
        f.push_str(&format!("[a{i}]"));
    }
    f.push_str(&format!("concat=n={n}:v=0:a=1[aout]"));
    f
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
        Err(e) if enc.is_hardware() && project.export.format == Format::Mp4 => {
            eprintln!("{} failed ({e:#}); retrying with libx264", enc.name());
            progress(0.0);
            export_with(project, out, crate::ffmpeg::Encoder::X264, &mut progress)
        }
        r => r,
    }
}

fn export_with(project: &Project, out: &Path, enc: crate::ffmpeg::Encoder, progress: &mut dyn FnMut(f64)) -> Result<ExportStats> {
    project.validate()?;
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

    // Decode the source at a high enough rate that sped-up clips still have
    // a frame for every output frame.
    let max_speed = project.clips.iter().map(|c| c.speed).fold(1.0, f64::max);
    let dec_fps = (info.fps.max(fps)).min(fps * max_speed).max(fps);
    let first_src = project.clips[0].src_start_ms;
    let (dw, dh) = renderer.decode_size();
    let mut reader = FrameReader::open(&project.source, first_src, dec_fps, dw, dh)?;
    let mut current: Option<Frame> = reader.next()?;

    let mut cmd = Command::new(ffmpeg::ffmpeg_bin());
    cmd.args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{ow}x{oh}"), "-r", &fps.to_string(), "-i", "-"]);
    let with_audio = s.format == Format::Mp4 && info.has_audio && !project.audio.mute;
    // Noise cleanup writes a cleaned track next to the output and uses it as
    // the audio input instead of the original.
    let mut audio_input = project.source.clone();
    let denoised = out.with_extension("denoised.wav");
    if with_audio && project.audio.denoise {
        crate::audio::denoise_file(&project.source, &denoised)?;
        audio_input = denoised.clone();
    }
    match s.format {
        Format::Mp4 => {
            if with_audio {
                cmd.arg("-i").arg(&audio_input);
                cmd.args(["-filter_complex", &audio_filter(project), "-map", "0:v", "-map", "[aout]", "-c:a", "aac", "-b:a", "160k"]);
            }
            cmd.args(enc.export_args(ow, oh, fps)).args(["-pix_fmt", "yuv420p", "-movflags", "+faststart", "-shortest"]);
        }
        Format::Gif => {
            cmd.args(["-vf", "split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a", "-loop", "0"]);
        }
    }
    cmd.arg(out);
    let mut enc = cmd.stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().context("failed to start ffmpeg encoder")?;
    let mut stdin = enc.stdin.take().unwrap();

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
        let frame = renderer.render(src, src_t);
        if stdin.write_all(&frame.data).is_err() {
            break;
        }
        progress((i + 1) as f64 / total as f64);
    }
    drop(stdin);
    let res = enc.wait_with_output()?;
    let _ = std::fs::remove_file(&denoised);
    if !res.status.success() {
        bail!("ffmpeg encode failed: {}", String::from_utf8_lossy(&res.stderr));
    }
    Ok(ExportStats { frames: total, width: ow, height: oh, seconds: out_dur / 1000.0 })
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
}
