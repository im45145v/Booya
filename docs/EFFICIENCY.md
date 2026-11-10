# Efficiency

Booya runs on laptops, often while a call or a browser is open, so it tries to use as little CPU, memory and battery as it can without changing what you get.

## What Booya does to save work

- **Hardware video encoding.** Recording and export use the Mac's media engine (`h264_videotoolbox`) on macOS and NVENC on NVIDIA machines. Booya checks once per run that the encoder actually opens, and falls back to software x264 if it does not. If a hardware export fails partway, it is redone in software automatically. Set `BOOYA_ENCODER=software` to force software.
- **Faster software export preset.** When software encoding is used, export uses x264 `veryfast` at CRF 18 instead of `medium`.
- **Decode only the pixels that are needed.** A Retina recording is decoded at the smallest size that still gives one source pixel per output pixel at the deepest zoom in the project. With no zoom, a 2880x1800 recording exported at 1080p is decoded at about 1680 wide. Zoomed projects decode at full size where needed, so zooms stay sharp. Blur boxes are scaled to match, so redaction is unchanged.
- **Decode, draw and encode at the same time.** Export used to decode a frame, draw it, hand it to the encoder and only then decode the next one. The screen and camera are now decoded on their own threads a few frames ahead, and finished frames go to the encoder from another thread, so all three stages overlap. The queues hold at most three frames each, so memory stays flat.
- **A leaner compositor loop.** The source column for each output column is worked out once per frame instead of once per pixel, the rounded corner distance is only computed near the card's edges, bilinear sampling reads pixels directly when it is not at the frame edge, and the background copy runs on all cores. The output is byte for byte the same (checked with a checksum over 90 rendered frames at 1080p and 4K before and after).
- **The drop shadow is painted once.** It used to be recomputed for every pixel of every frame. It is now baked into the background when the renderer is created, and each frame only visits the rows and columns the recording card covers. The output is pixel identical (checked by comparing raw RGBA hashes of preview frames before and after).
- **Lighter previews.** Preview PNGs use fast compression, play mode renders at 360p instead of 540p, and playback stops when the window is hidden.
- **Idle mouse tracking on macOS.** The cursor tracker polls at 60 Hz while the mouse moves and drops to 20 Hz after half a second of stillness.
- **Smaller binaries.** Release builds use full LTO, one codegen unit and stripped symbols.

## Measured results

Measured on the Linux build machine (8 vCPUs, software x264, no GPU encoder) on Oct 27, 2026, comparing v0.1.2 code (before) with v0.1.3 code (after), both release builds. CPU time is user plus system time from `time`. Each export was run twice and the numbers below are from the second run.

| Task | Before | After | Change |
| --- | --- | --- | --- |
| Export a 20 s 2880x1800 30 fps recording to 1080p MP4, wall time | 17.4 s | 11.6 s | 33% faster |
| Same export, CPU time | 90.3 s | 58.5 s | 35% less CPU |
| Same export, output size | 13.5 MB | 6.6 MB | 51% smaller |
| Export the built-in 1920x1080 sample (with zooms) to 1080p, wall time | 7.7 s | 7.2 s | 7% faster |
| Same sample export, CPU time | 41.3 s | 32.9 s | 20% less CPU |
| x264 alone, 20 s of 1080p30, `medium` vs `veryfast` at CRF 18, CPU time | 29.1 s | 15.2 s | 48% less CPU |
| CLI binary size (`booya`) | 18.5 MB | 13.0 MB | 30% smaller |
| App binary size (`booya-app`) | 29.3 MB | 18.8 MB | 36% smaller |

Quality check: PSNR of the new export against the old one is 41.9 dB for the Retina test clip and 55.2 dB for the sample. Above 40 dB the difference is not visible.

The Retina test clip is synthetic (ffmpeg `testsrc2`). Real screen recordings compress differently, so the file size change will vary.

Not measured here: the hardware encoders. The build machine has no VideoToolbox or NVENC, so the CPU and battery savings from hardware encoding on a Mac are expected but not yet measured. On Apple Silicon, H.264 encoding moves to the media engine, which normally leaves the CPU close to idle during recording.

### Export speed, November 2026

Measured on the same kind of 8 vCPU Linux box (software x264, no GPU encoder) on Nov 10, 2026, exporting the built-in 12 second sample with `cargo run --release -p booya-core --example export_bench -- <dir> <quality>`. The box was shared with other jobs, so each build was run several times, alternating, and the best run of each is shown.

| Task | Before | After | Change |
| --- | --- | --- | --- |
| Sample to 1080p MP4, wall time | 7.6 s | 5.1 s | 33% faster |
| Sample to 4K MP4, wall time | 25.8 s | 14.1 s | 45% faster |
| Drawing one 1080p frame, no decode or encode | 10.1 ms | 7.5 ms | 26% faster |
| Drawing one 4K frame, no decode or encode | 33.2 ms | 24.5 ms | 26% faster |

Not measured here: exports with a webcam (the camera now decodes on its own thread too, so they should gain at least as much) and the hardware encoders on a Mac or an NVIDIA card, where the encoder is no longer the slow part and the overlap matters more.

## Left for later

- Skip re-encoding entirely when a project has no edits and no styling.
- Whisper: voice activity detection to skip silence, and caching transcripts by file hash so reruns are free.
- Text recognition for blur: sample fewer frames when the screen has not changed.
- Measure recording CPU and energy on an M-series Mac with `powermetrics`.
