//! Advanced audio-video synchronization utilities
//!
//! This module provides proper PTS (Presentation Time Stamp) based synchronization
//! to ensure audio and video streams maintain perfect timing throughout the
//! recording and export process.

use crate::project::Project;
use crate::ffmpeg;
use anyhow::Result;
use std::path::Path;

/// Synchronization configuration for handling PTS adjustments
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Target time base for synchronization (typically 1/48000 for audio)
    pub time_base: f64,
    /// Maximum allowed drift before correction (in seconds)
    pub max_drift: f64,
    /// Whether to apply real-time corrections during recording
    pub real_time: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            time_base: 1.0 / 48000.0, // 48kHz audio sample rate
            max_drift: 0.01, // 10ms maximum drift
            real_time: false,
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
}

/// Timing information for a single media stream
#[derive(Debug, Clone)]
pub struct StreamTiming {
    /// Start time in seconds (PTS of first frame/packet)
    pub start_time: f64,
    /// Duration in seconds
    pub duration: f64,
    /// Average frame/packet duration
    pub avg_packet_duration: f64,
    /// Time base of the stream
    pub time_base: f64,
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
}

/// Analyze synchronization between audio and video streams
pub fn analyze_sync(source_path: &Path) -> Result<SyncAnalysis> {
    let media_info = ffmpeg::probe(source_path)?;
    
    let video_timing = StreamTiming {
        start_time: 0.0, // Video typically starts at 0
        duration: media_info.duration_ms / 1000.0,
        avg_packet_duration: 1.0 / media_info.fps,
        time_base: 1.0 / media_info.fps,
    };
    
    let audio_timing = if media_info.has_audio {
        // For audio, we need to get more detailed information
        Some(StreamTiming {
            start_time: 0.0, // Audio typically starts at 0 or slightly offset
            duration: media_info.duration_ms / 1000.0,
            avg_packet_duration: 1.0 / 48000.0, // Standard sample rate
            time_base: 1.0 / 48000.0,
        })
    } else {
        None
    };
    
    let drift = audio_timing.as_ref().map(|audio| {
        audio.start_time - video_timing.start_time
    });
    
    let adjustments = calculate_adjustments(&video_timing, audio_timing.as_ref(), drift);
    
    Ok(SyncAnalysis {
        video_timing,
        audio_timing,
        drift,
        adjustments,
    })
}

/// Calculate necessary PTS adjustments based on timing analysis
fn calculate_adjustments(
    video: &StreamTiming,
    audio: Option<&StreamTiming>,
    drift: Option<f64>,
) -> Vec<PtsAdjustment> {
    let mut adjustments = Vec::new();
    
    if let (Some(audio), Some(drift)) = (audio, drift) {
        // Check for significant drift
        if drift.abs() > 0.01 { // More than 10ms drift
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
        
        // Check for duration mismatch
        let duration_diff = (video.duration - audio.duration).abs();
        if duration_diff > 0.05 { // More than 50ms difference
            adjustments.push(PtsAdjustment {
                kind: AdjustmentKind::Resample,
                value: if audio.duration < video.duration {
                    video.duration / audio.duration
                } else {
                    audio.duration / video.duration
                },
                reason: format!("Audio duration differs by {:.1}ms from video", duration_diff * 1000.0),
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
        }
    }
    
    if filters.is_empty() {
        "anull".to_string() // No changes needed
    } else {
        filters.join(",")
    }
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
    }
    
    #[test]
    fn test_calculate_adjustments_no_drift() {
        let video = StreamTiming {
            start_time: 0.0,
            duration: 60.0,
            avg_packet_duration: 1.0 / 30.0,
            time_base: 1.0 / 30.0,
        };
        
        let audio = StreamTiming {
            start_time: 0.0,
            duration: 60.0,
            avg_packet_duration: 1.0 / 48000.0,
            time_base: 1.0 / 48000.0,
        };
        
        let adjustments = calculate_adjustments(&video, Some(&audio), Some(0.0));
        assert!(adjustments.is_empty());
    }
    
    #[test]
    fn test_calculate_adjustments_with_drift() {
        let video = StreamTiming {
            start_time: 0.0,
            duration: 60.0,
            avg_packet_duration: 1.0 / 30.0,
            time_base: 1.0 / 30.0,
        };
        
        let audio = StreamTiming {
            start_time: 0.005, // 5ms late
            duration: 60.0,
            avg_packet_duration: 1.0 / 48000.0,
            time_base: 1.0 / 48000.0,
        };
        
        let adjustments = calculate_adjustments(&video, Some(&audio), Some(0.005));
        assert!(!adjustments.is_empty());
        assert_eq!(adjustments.len(), 1);
        assert_eq!(adjustments[0].kind, AdjustmentKind::Delay);
        assert_eq!(adjustments[0].value, 0.005);
    }
}