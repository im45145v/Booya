# Third party notices

DemoLens itself is MIT licensed. The installers also contain the programs below.

## FFmpeg

The desktop installers include static builds of `ffmpeg` and `ffprobe` from the FFmpeg project (https://ffmpeg.org), installed next to the app as `demolens-ffmpeg` and `demolens-ffprobe`. These builds include GPL components (for example x264 and x265), so they are distributed under the GNU General Public License version 2 or later. See https://ffmpeg.org/legal.html.

- Linux and Windows builds: https://github.com/BtbN/FFmpeg-Builds (source and build scripts in that repository)
- macOS builds: https://ffmpeg.martin-riedl.de (source and build scripts linked from that site)

DemoLens runs FFmpeg as a separate program and does not link to it. You can replace the bundled copy with your own: set `DEMOLENS_FFMPEG` and `DEMOLENS_FFPROBE`, or delete the bundled files and DemoLens will use the ffmpeg on your PATH. The exact download script is `scripts/fetch-ffmpeg.sh`.

FFmpeg is a trademark of Fabrice Bellard, originator of the FFmpeg project.

## Optional AI models

Models are downloaded on demand, never bundled, and keep their own licenses (Whisper: MIT, PaddleOCR models: Apache 2.0 / CC-BY-SA-4.0, Qwen2.5: Apache 2.0, llama.cpp: MIT).
