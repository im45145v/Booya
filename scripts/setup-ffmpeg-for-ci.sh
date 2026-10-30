#!/usr/bin/env bash
# Setup script for ensuring ffmpeg is available in CI/CD environments
# This script ensures that the locally fetched ffmpeg binaries are used

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BINARIES_DIR="$PROJECT_ROOT/src-tauri/binaries"

echo "Setting up ffmpeg for CI/CD environment..."

# Check if ffmpeg is already in PATH and working
if command -v ffmpeg >/dev/null 2>&1; then
    echo "ffmpeg already available in PATH:"
    ffmpeg -version | head -1
else
    # Run the fetch script to ensure binaries are present
    echo "Fetching ffmpeg binaries..."
    "$PROJECT_ROOT/scripts/fetch-ffmpeg.sh"
    
    # Find the appropriate binary for this platform
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS
        if [[ "$(uname -m)" == "arm64" ]]; then
            FFMPEG_BIN="$BINARIES_DIR/booya-ffmpeg-aarch64-apple-darwin"
        else
            FFMPEG_BIN="$BINARIES_DIR/booya-ffmpeg-x86_64-apple-darwin"
        fi
    elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
        # Linux
        FFMPEG_BIN="$BINARIES_DIR/booya-ffmpeg-x86_64-unknown-linux-gnu"
    elif [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
        # Windows
        FFMPEG_BIN="$BINARIES_DIR/booya-ffmpeg-x86_64-pc-windows-msvc.exe"
    else
        echo "Unsupported OS: $OSTYPE"
        exit 1
    fi
    
    # Verify the binary exists
    if [[ -f "$FFMPEG_BIN" ]]; then
        echo "Found ffmpeg binary at: $FFMPEG_BIN"
        # Add to PATH for the current session
        export PATH="$BINARIES_DIR:$PATH"
        echo "Added $BINARIES_DIR to PATH"
        
        # Verify it works
        if command -v ffmpeg >/dev/null 2>&1; then
            echo "ffmpeg is now available:"
            ffmpeg -version | head -1
        else
            echo "Failed to make ffmpeg available in PATH"
            exit 1
        fi
    else
        echo "ffmpeg binary not found at: $FFMPEG_BIN"
        exit 1
    fi
fi

echo "ffmpeg setup complete!"