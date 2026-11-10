# Sound against picture: what was measured, on which OS

You reported audio lagging the picture on a Mac. The question was whether that is a Mac problem or a general one. A test signal answers it: a clip with a flash every second and a beep at the same instants. `booya syncprobe` finds the flashes and beeps in a file and prints how far the sound is from the picture, how that changes over the clip (drift) and how much it wobbles (jitter). It runs on CI on every push.

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

## November 2026: where the voice really starts, and a camera that keeps time

Measured on Linux (Xvfb, PulseAudio null sink, 60 s of flashes and beeps, one run each), through `capture::start` and `stop` like the app:

| | sound vs picture | drift over 60 s | jitter |
|---|---|---|---|
| before (main at f1d4910) | +53 ms | 0 ms | 0 ms |
| after | -1 ms | 0 ms | 0 ms |

Both numbers include the test's own start skew (the beep is a separate process), so the honest reading is "about 50 ms better", not "perfect". The test used to play the beep with PulseAudio's default buffer, which put it about a second late; with flashes every second that looked like a plausible offset. It now plays with a 20 ms buffer and the CLI holds the capture offset to 100 ms.

What changed:

- **Microphone start.** The microphone file is stamped with the moment its first packet was read. That packet was captured earlier, by at least its own length plus whatever the device buffered before the first read, and that amount changes from one recording to the next. This is the "sometimes early, sometimes on time" in issue #7. The microphone recorder now also writes a packet log (`recording.mic.timing`, read time and size of every packet). At the end, the start is worked out from all packets: read time minus samples so far, at its smallest. In the run above the voice started 69 ms before its first packet was read.
- **Microphone clock.** The same log gives the microphone's real rate against the wall clock, over the whole take. Before, drift was only corrected when the voice came out 2.5% to 20% short of the video, and the video's length was the yardstick even though the microphone is stopped before the screen. Now a difference over 0.05% is corrected (up to 1% by relabelling the sample rate, which nobody can hear, beyond that with a tempo change). On this machine the clock was 0.023% slow, under the threshold.
- **Camera.** Camera frames were spaced at exactly 30 fps after the first one. A built-in camera that drops to 15 or 24 fps in a dim room then played up to twice as fast and ran ahead of the voice; a DirectShow camera opened in a non 30 fps default mode did the same. Every frame now keeps its wall-clock time, and only frames that arrive in a burst are spread out.
- **Clicks and zoom.** The event log was moved to the first frame only when that frame came more than 150 ms after the clock started, and on Linux and Windows a recording without sound had no such time at all. Both now always use the real lead-in.

## What is not tested

- macOS capture sync, and any camera. CI runners cannot capture a screen, a microphone or a camera. Mac numbers need a recording on a real Mac. Run `booya verify recording.mp4` and send the output.
- Windows microphone capture.
- Linux with both microphone and system audio. That path still mixes them in one process.
- The preview fix needs a window. It was tested as logic (`tests/js/avsync.test.js`), not by eye.
