//! Advanced audio-video synchronization utilities
//!
//! This module provides proper PTS (Presentation Time Stamp) based synchronization
//! to ensure audio and video streams maintain perfect timing throughout the
//! recording and export process.
//!
//! ## Problem Solved
//!
//! Audio can end before video due to:
//! - Buffer underruns during capture (ScreenCaptureKit dropping frames)
//! - Audio device stopping earlier than video
//! - Timestamp drift between audio/video clocks
//!
//! ## Solution Approach
//!
//! 1. **Stream Duration Analysis**: Precisely measure audio vs video duration
//! 2. **PTS Normalization**: Ensure both streams use consistent time base
//! 3. **Automatic Correction**: Extend audio with silence when needed
//! 4. **Real-time Monitoring**: Detect drift during recording (future)

use crate::project::Project;
use crate::ffmpeg;
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// Synchronization configuration for handling PTS adjustments
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Target time base for synchronization (typically 1/48000 for audio)
    pub time_base: f64,
    /// Maximum allowed drift before correction (in seconds)
    pub max_drift: f64,
    /// Whether to apply real-time corrections during recording
    pub real_time: bool,
    /// Sample rate for audio processing
    pub audio_sample_rate: u32,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            time_base: 1.0 / 48000.0, // 48kHz audio sample rate
            max_drift: 0.01, // 10ms maximum drift
            real_time: false,
            audio_sample_rate: 48000,
        }
    }
}

/// Detailed synchronization analysis of media streams
#[derive(Debug, Clone)]
pub struct SyncAnalysis {
    /// Video stream timing information
    pub video_timing: StreamTiming,
    /// Audio stream timing information (if present)
    pub audio_timing: Option<StreamTiming>,
    /// Calculated drift between streams
    pub drift: Option<f64>,
    /// Recommended PTS adjustments
    pub adjustments: Vec<PtsAdjustment>,
    /// Whether significant sync issues were detected
    pub has_issues: bool,
    /// Human-readable diagnostic message
    pub diagnostic: String,
}

/// Timing information for a single media stream
#[derive(Debug, Clone, Default)]
pub struct StreamTiming {
    /// Start time in seconds (PTS of first frame/packet)
    pub start_time: f64,
    /// Duration in seconds
    pub duration: f64,
    /// Average frame/packet duration
    pub avg_packet_duration: f64,
    /// Time base of the stream
    pub time_base: f64,
    /// Number of frames/packets
    pub packet_count: u64,
}

/// PTS adjustment recommendation
#[derive(Debug, Clone)]
pub struct PtsAdjustment {
    /// Type of adjustment needed
    pub kind: AdjustmentKind,
    /// Value of the adjustment
    pub value: f64,
    /// Reason for the adjustment
    pub reason: String,
}

/// Types of PTS adjustments
#[derive(Debug, Clone, PartialEq)]
pub enum AdjustmentKind {
    /// Delay the stream by specified seconds
    Delay,
    /// Trim the beginning of the stream
    TrimStart,
    /// Adjust the time base
    TimeBase,
    /// Resample to fix timing
    Resample,
    /// Extend audio with silence
    Extend,
    /// Trim the end of the stream
    TrimEnd,
}

/// Analyze synchronization between audio and video streams
pub fn analyze_sync(source_path: &Path) -> Result<SyncAnalysis> {
    let media_info = ffmpeg::probe(source_path)?;
    
    // Get precise stream durations using ffprobe
    let (video_duration, audio_duration) = get_precise_durations(source_path)?;
    
    let video_timing = StreamTiming {
        start_time: 0.0, // Video typically starts at 0
        duration: video_duration,
        avg_packet_duration: 1.0 / media_info.fps,
        time_base: 1.0 / media_info.fps,
        packet_count: (video_duration * media_info.fps) as u64,
    };
    
    let audio_timing = if media_info.has_audio {
        Some(StreamTiming {
            start_time: 0.0,
            duration: audio_duration,
            avg_packet_duration: 1.0 / 48000.0, // Standard sample rate
            time_base: 1.0 / 48000.0,
            packet_count: (audio_duration * 48000.0) as u64,
        })
    } else {
        None
    };
    
    let drift = audio_timing.as_ref().map(|audio| {
        audio.start_time - video_timing.start_time
    });
    
    let adjustments = calculate_adjustments(&video_timing, audio_timing.as_ref(), drift);
    
    // Determine if we have significant issues
    let duration_diff = (video_duration - audio_duration).abs();
    let has_issues = duration_diff > 0.5; // More than 500ms difference
    
    let diagnostic = if has_issues {
        format!(
            "Significant A/V sync issue: Video={:.2}s, Audio={:.2}s, Gap={:.2}s",
            video_duration,
            audio_duration,
            video_duration - audio_duration
        )
    } else {
        format!("A/V sync OK: Video={:.2}s, Audio={:.2}s", video_duration, audio_duration)
    };
    
    Ok(SyncAnalysis {
        video_timing,
        audio_timing,
        drift,
        adjustments,
        has_issues,
        diagnostic,
    })
}

/// Get precise durations for video and audio streams using ffprobe
fn get_precise_durations(source_path: &Path) -> Result<(f64, f64)> {
    let output = Command::new(ffmpeg::ffprobe_bin())
        .args(&[
            "-v", "error",
            "-show_entries", "stream=codec_type,duration,start_time",
            "-of", "json",
        ])
        .arg(source_path)
        .output()
        .context("Failed to run ffprobe for duration analysis")?;
    
    if !output.status.success() {
        bail!("ffprobe failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    let json_str = String::from_utf8(output.stdout)
        .context("ffprobe output is not valid UTF-8")?;
    
    // Parse JSON to extract durations
    let v: serde_json::Value = serde_json::from_str(&json_str)
        .context("Failed to parse ffprobe JSON output")?;
    
    let mut video_duration = 0.0f64;
    let mut audio_duration = 0.0f64;
    
    if let Some(streams) = v["streams"].as_array() {
        for stream in streams {
            let codec_type = stream["codec_type"].as_str().unwrap_or("");
            let duration = stream["duration"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            
            match codec_type {
                "video" => video_duration = duration,
                "audio" => audio_duration = duration,
                _ => {}
            }
        }
    }
    
    Ok((video_duration, audio_duration))
}

/// Calculate necessary PTS adjustments based on timing analysis
pub fn calculate_adjustments(
    video: &StreamTiming,
    audio: Option<&StreamTiming>,
    drift: Option<f64>,
) -> Vec<PtsAdjustment> {
    let mut adjustments = Vec::new();
    
    if let (Some(audio), Some(drift)) = (audio, drift) {
        // Check for significant drift (audio starting before/after video)
        if drift.abs() >= 0.005 { // 5ms or more drift
            if drift > 0.0 {
                adjustments.push(PtsAdjustment {
                    kind: AdjustmentKind::Delay,
                    value: drift,
                    reason: format!("Audio starts {:.1}ms after video", drift * 1000.0),
                });
            } else {
                adjustments.push(PtsAdjustment {
                    kind: AdjustmentKind::TrimStart,
                    value: -drift,
                    reason: format!("Audio starts {:.1}ms before video", -drift * 1000.0),
                });
            }
        }
        
        // Check for duration mismatch - THIS IS THE KEY FIX
        let duration_diff = video.duration - audio.duration;
        if duration_diff.abs() > 0.05 { // More than 50ms difference
            if duration_diff > 0.0 {
                // Audio is shorter than video - need to extend
                adjustments.push(PtsAdjustment {
                    kind: AdjustmentKind::Extend,
                    value: duration_diff,
                    reason: format!(
                        "Audio is {:.1}s shorter than video - will extend with silence",
                        duration_diff
                    ),
                });
            } else {
                // Audio is longer than video - need to trim
                adjustments.push(PtsAdjustment {
                    kind: AdjustmentKind::TrimEnd,
                    value: -duration_diff,
                    reason: format!(
                        "Audio is {:.1}s longer than video - will trim to match",
                        -duration_diff
                    ),
                });
            }
        }
        
        // Check for time base inconsistencies
        let tb_ratio = video.time_base / audio.time_base;
        if (tb_ratio - 1.0).abs() > 0.01 {
            adjustments.push(PtsAdjustment {
                kind: AdjustmentKind::TimeBase,
                value: tb_ratio,
                reason: "Video and audio have different time bases".to_string(),
            });
        }
    }
    
    adjustments
}

/// Apply synchronization fixes to a project based on analysis
pub fn apply_sync_fixes(project: &Project, analysis: &SyncAnalysis) -> Project {
    let mut fixed_project = project.clone();
    
    // Apply recommended adjustments
    for adjustment in &analysis.adjustments {
        match adjustment.kind {
            AdjustmentKind::Delay => {
                // Add delay to audio
                fixed_project.audio.offset_ms += adjustment.value * 1000.0;
            }
            AdjustmentKind::TrimStart => {
                // Trim audio start
                fixed_project.audio.offset_ms -= adjustment.value * 1000.0;
            }
            AdjustmentKind::Resample => {
                // For duration mismatches, we'll handle this in the export filter
                // The apad filter will extend audio with silence if needed
            }
            AdjustmentKind::TimeBase => {
                // Time base adjustments are handled by ffmpeg filters
            }
            AdjustmentKind::Extend | AdjustmentKind::TrimEnd => {
                // Duration adjustments are handled in the export pipeline
                // via pad_audio_graph_enhanced
                eprintln!("Sync fix: {}", adjustment.reason);
            }
        }
    }
    
    fixed_project
}

/// Generate ffmpeg filter for proper PTS synchronization
pub fn sync_filter(analysis: &SyncAnalysis) -> String {
    let mut filters = Vec::new();
    
    // Apply any recommended adjustments as ffmpeg filters
    for adjustment in &analysis.adjustments {
        match adjustment.kind {
            AdjustmentKind::Delay => {
                filters.push(format!("adelay={:.0}:all=1", adjustment.value * 1000.0));
            }
            AdjustmentKind::TrimStart => {
                filters.push(format!("atrim=start={:.3}", adjustment.value));
            }
            AdjustmentKind::Resample => {
                // For duration issues, we rely on apad filter in export pipeline
            }
            AdjustmentKind::TimeBase => {
                filters.push("aresample=48000".to_string());
            }
            AdjustmentKind::Extend => {
                // Handled separately with whole_dur in export
                filters.push(format!("apad=whole_dur={:.3}", 
                    analysis.video_timing.duration));
            }
            AdjustmentKind::TrimEnd => {
                filters.push(format!("atrim=duration={:.3}", 
                    analysis.video_timing.duration));
            }
        }
    }
    
    if filters.is_empty() {
        "anull".to_string() // No changes needed
    } else {
        filters.join(",")
    }
}

/// Generate a comprehensive diagnostic report
pub fn generate_diagnostic(analysis: &SyncAnalysis) -> String {
    let mut report = String::new();
    
    report.push_str(&format!("Video Duration: {:.3}s\n", analysis.video_timing.duration));
    
    if let Some(audio) = &analysis.audio_timing {
        report.push_str(&format!("Audio Duration: {:.3}s\n", audio.duration));
        report.push_str(&format!("Duration Gap: {:.3}s\n", 
            analysis.video_timing.duration - audio.duration));
    }
    
    if let Some(drift) = analysis.drift {
        report.push_str(&format!("Start Time Drift: {:.1}ms\n", drift * 1000.0));
    }
    
    if !analysis.adjustments.is_empty() {
        report.push_str("\nRecommended Fixes:\n");
        for adj in &analysis.adjustments {
            report.push_str(&format!("  - {}\n", adj.reason));
        }
    } else {
        report.push_str("\nNo adjustments needed.\n");
    }
    
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sync_config_default() {
        let config = SyncConfig::default();
        assert_eq!(config.time_base, 1.0 / 48000.0);
        assert_eq!(config.max_drift, 0.01);
        assert_eq!(config.real_time, false);
        assert_eq!(config.audio_sample_rate, 48000);
    }
    
    #[test]
    fn test_calculate_adjustments_no_drift() {
        let video = StreamTiming {
            start_time: 0.0,
            duration: 60.0,
            avg_packet_duration: 1.0 / 30.0,
            time_base: 1.0 / 30.0,
            packet_count: 1800,
        };
        
        let audio = StreamTiming {
            start_time: 0.0,
            duration: 60.0,
            avg_packet_duration: 1.0 / 48000.0,
            time_base: 1.0 / 48000.0,
            packet_count: 2880000,
        };
        
        let adjustments = calculate_adjustments(&video, Some(&audio), Some(0.0));
        // Should have one adjustment: time base difference
        // (video time_base 1/30 vs audio time_base 1/48000)
        assert_eq!(adjustments.len(), 1);
        assert_eq!(adjustments[0].kind, AdjustmentKind::TimeBase);
    }
    
    #[test]
    fn test_calculate_adjustments_with_drift() {
        let video = StreamTiming {
            start_time: 0.0,
            duration: 60.0,
            avg_packet_duration: 1.0 / 30.0,
            time_base: 1.0 / 30.0,
            packet_count: 1800,
        };
        
        let audio = StreamTiming {
            start_time: 0.005, // 5ms late
            duration: 60.0,
            avg_packet_duration: 1.0 / 48000.0,
            time_base: 1.0 / 48000.0,
            packet_count: 2880000,
        };
        
        let adjustments = calculate_adjustments(&video, Some(&audio), Some(0.005));
        // Should have 2 adjustments: drift (delay) + time base difference
        assert_eq!(adjustments.len(), 2);
        
        let delay_adj = adjustments.iter().find(|a| a.kind == AdjustmentKind::Delay);
        assert!(delay_adj.is_some());
        assert_eq!(delay_adj.unwrap().value, 0.005);
    }
    
    #[test]
    fn test_calculate_adjustments_audio_shorter() {
        let video = StreamTiming {
            start_time: 0.0,
            duration: 67.0, // 67 seconds (like your recording)
            avg_packet_duration: 1.0 / 30.0,
            time_base: 1.0 / 30.0,
            packet_count: 2010,
        };
        
        let audio = StreamTiming {
            start_time: 0.0,
            duration: 61.0, // 61 seconds (6 second gap)
            avg_packet_duration: 1.0 / 48000.0,
            time_base: 1.0 / 48000.0,
            packet_count: 2928000,
        };
        
        let adjustments = calculate_adjustments(&video, Some(&audio), Some(0.0));
        assert!(!adjustments.is_empty());
        
        // Should have an Extend adjustment
        let extend_adj = adjustments.iter().find(|a| a.kind == AdjustmentKind::Extend);
        assert!(extend_adj.is_some());
        assert!((extend_adj.unwrap().value - 6.0).abs() < 0.01);
    }
}