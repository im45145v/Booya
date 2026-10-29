//! Runs the part of transcription that follows whisper (retiming, voice
//! anchoring, caption grouping) on a recording and the words whisper gave.
//! Usage: postproc <video> <words.json>
use booya_ai::transcribe;
use booya_core::{audio, captions, project::CaptionWord};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a[1] == "dump" {
        return dump(&a[2], &a[3]);
    }
    let samples = audio::decode_mono(std::path::Path::new(&a[1]), 16_000).unwrap();
    let mut words: Vec<CaptionWord> = serde_json::from_str(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let spans = transcribe::speech_spans(&samples, 16_000);
    let voiced: Vec<(f64, f64)> = spans.iter().map(|&(x, y)| (x as f64 / 16.0, y as f64 / 16.0)).collect();
    let tight = transcribe::voice_onsets(&samples, 16_000);
    println!("spans  {:?}", voiced.iter().map(|v| (v.0.round(), v.1.round())).collect::<Vec<_>>());
    println!("onsets {:?}", tight.iter().map(|v| (v.0.round(), v.1.round())).collect::<Vec<_>>());
    let show = |tag: &str, w: &[CaptionWord]| println!("{tag}: {:?}", w.iter().map(|w| (w.text.as_str(), w.start_ms.round(), w.end_ms.round())).collect::<Vec<_>>());
    captions::snap_to_speech(&mut words, &audio::detect_silences(&samples, 16_000, -38.0, 250.0));
    show("snapped", &words);
    if std::env::var_os("OLD").is_some() {
        transcribe::retime_implausible(&mut words, &tight);
        transcribe::anchor_to_voice(&mut words, &tight);
    } else {
        transcribe::fix_times(&mut words, &tight, &tight);
    }
    show("fixed", &words);
    for c in captions::words_to_captions(&words, 42, 900.0) {
        println!("caption {:.1}-{:.1} {}", c.start_ms / 1000.0, c.end_ms / 1000.0, c.text);
    }
}

/// `postproc dump <video> <out.wav>`: the audio exactly as whisper gets it
/// (levelled and with long silences squeezed), as 16 bit mono WAV.
#[allow(dead_code)]
fn dump(video: &str, out: &str) {
    let a = audio::decode_mono(std::path::Path::new(video), 16_000).unwrap();
    let a = transcribe::normalise_for_speech(a);
    let spans = transcribe::speech_spans(&a, 16_000);
    let (c, map) = transcribe::compact(&a, &spans, 16_000);
    let mut b = Vec::new();
    for s in &c {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + b.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&16000u32.to_le_bytes());
    w.extend_from_slice(&32000u32.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(b.len() as u32).to_le_bytes());
    w.extend_from_slice(&b);
    std::fs::write(out, w).unwrap();
    std::fs::write(format!("{out}.map.json"), serde_json::to_string(&map).unwrap()).unwrap();
}
