#!/usr/bin/env bash
# Downloads static ffmpeg and ffprobe for this OS into src-tauri/binaries,
# named the way Tauri expects sidecars (name-<target triple>[.exe]).
# Builds: BtbN/FFmpeg-Builds (Linux, Windows, GPL) and ffmpeg.martin-riedl.de (macOS, GPL).
set -euo pipefail
out="$(cd "$(dirname "$0")/.." && pwd)/src-tauri/binaries"
mkdir -p "$out"
triple=$(rustc -vV | sed -n 's/^host: //p')
tmp=$(mktemp -d)
cd "$tmp"
case "$triple" in
  *linux*)
    curl -fsSL -o f.tar.xz https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz
    tar -xf f.tar.xz
    d=$(ls -d ffmpeg-*/bin)
    cp "$d/ffmpeg" "$out/booya-ffmpeg-$triple"; cp "$d/ffprobe" "$out/booya-ffprobe-$triple"
    ;;
  *apple-darwin*)
    arch=arm64; [[ "$triple" == x86_64* ]] && arch=amd64
    for b in ffmpeg ffprobe; do
      curl -fsSL -o $b.zip "https://ffmpeg.martin-riedl.de/redirect/latest/macos/$arch/release/$b.zip"
      unzip -oq $b.zip
      cp $b "$out/booya-$b-$triple"
    done
    ;;
  *windows*)
    curl -fsSL -o f.zip https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip
    unzip -q f.zip
    d=$(ls -d ffmpeg-*/bin)
    cp "$d/ffmpeg.exe" "$out/booya-ffmpeg-$triple.exe"; cp "$d/ffprobe.exe" "$out/booya-ffprobe-$triple.exe"
    ;;
  *) echo "no ffmpeg build for $triple"; exit 1 ;;
esac
chmod +x "$out"/* || true
ls -la "$out"
"$out/booya-ffmpeg-$triple"* -hide_banner -version | head -1
