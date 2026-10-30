//! Test for audio-video synchronization issues

use booya_core::export::*;
use booya_core::project::*;

#[test]
fn test_audio_compensation() {
    // Create a project with video longer than audio
    let mut project = Project::default();
    project.duration_ms = 67000.0; // 1:07
    
    // Add a clip that only produces 61 seconds of audio
    let clip = Clip {
        src_start_ms: 0.0,
        src_end_ms: 61000.0, // 1:01 of source
        speed: 1.0,
        mute: false,
    };
    project.clips.push(clip);
    
    // Test compensation
    let compensated = compensate_audio_duration(&project, 67000.0);
    
    // Ensure we still have the same clip structure
    assert_eq!(compensated.clips.len(), 1);
    
    // Ensure fade out was applied
    assert!(compensated.audio.fade_out_ms >= 50.0);
    
    println!("Audio compensation test passed");
}