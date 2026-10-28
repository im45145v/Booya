# Sound against picture: what was measured, on which OS

You reported audio lagging the picture on a Mac. The question was whether that is a Mac problem or a general one. A test signal answers it: a clip with a flash every second and a beep at the same instants. `demolens syncprobe` finds the flashes and beeps in a file and prints how far the sound is from the picture, how that changes over the clip (drift) and how much it wobbles (jitter). It runs on CI on every push.

## Results (CI run of commit 5f48afb, 28 October 2026)

The file stages gave identical numbers on Ubuntu, macOS and Windows, because they are ffmpeg on files. Negative means the sound is early.

| stage | sound vs picture | drift over the clip | jitter | result |
|---|---|---|---|---|
| reference signal | 0 ms | 0 ms | 0 ms | ok |
| editor preview proxy (video and sound) | +4 ms | 0 ms | 0 ms | ok |
| export, styled | -16 ms | +1 ms | 2 ms | ok |
| export, copy | -16 ms | +1 ms | 2 ms | ok |
| export after cutting the first 3.5 s | -17 ms | +4 ms | 1 ms | ok |
| recording finish with a microphone that opened 350 ms late | 0 ms | 0 ms | 0 ms | ok |

Budget used: within 50 ms offset and 25 ms drift. The 16 ms the export leads is constant and below what anyone can see. It is probably AAC priming. It was not chased further.

Recording on Linux, in a virtual display with a virtual microphone (CI job `capture-sync`):

| capture method | sound vs picture | drift | jitter |
|---|---|---|---|
| screen and microphone as separate files (now used on every OS) | -118 ms | 0 ms | 0 ms |
| screen and microphone in one ffmpeg (what Linux and Windows did before) | -471 ms | 0 ms | 16 ms |

The test starts the two inputs one after another, so part of -118 ms is the test's own skew. The 353 ms improvement between the two methods is the evidence.

Windows screen clock (gdigrab, video only, not audio): 14 flashes, mean period 1.0014 s, worst deviation 38 ms. So the Windows picture has a steady clock. Audio capture on Windows has no CI test, because the runner has no sound device.

## Verdict

It is not Mac only.

- The editor preview clock was the same on every OS. It followed the video, not the sound, and could drift. `ui/avsync.js` now follows the sound.
- The "microphone in the same ffmpeg as the screen" design lost sound alignment on Linux in the test, and Windows used the same design. Both now record the microphone in its own process with wall clock stamps and line it up at the end, the way macOS has since 0.4.2.
- The public record agrees that this problem is cross platform: Cap #382 and #593 are about export sync, and Screen Studio's own developer has written up a sync bug.

## What is not tested

- macOS capture sync. CI runners cannot capture a screen or microphone. Mac numbers need a recording on a real Mac. Run `demolens verify recording.mp4` and send the output.
- Windows microphone capture.
- Linux with both microphone and system audio. That path still mixes them in one process.
- The preview fix needs a window. It was tested as logic (`tests/js/avsync.test.js`), not by eye.
