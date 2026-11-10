# One command setup for working on Booya on Windows 10 or 11.
#
#   powershell -ExecutionPolicy Bypass -File scripts\dev-setup.ps1          install what's missing
#   powershell -ExecutionPolicy Bypass -File scripts\dev-setup.ps1 -Check   only report what's missing
#
# Uses winget, which ships with Windows 11 and current Windows 10.
param([switch]$Check)
$ErrorActionPreference = 'Stop'
$missing = @()

function Have($cmd) { [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

function Need($label, $cmd, $wingetId) {
  if (Have $cmd) { Write-Host "${label}: installed."; return }
  $script:missing += $label
  if ($Check) { Write-Host "${label}: missing."; return }
  if (-not (Have 'winget')) { Write-Host "${label}: missing, and winget isn't available. Install it by hand."; return }
  Write-Host "${label}: installing with winget ($wingetId)..."
  winget install --id $wingetId -e --accept-source-agreements --accept-package-agreements
}

Write-Host "Booya dev setup (Windows)"
Write-Host ""

# The C++ build tools are needed by Rust (MSVC linker) and by whisper.cpp.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$hasVc = (Test-Path $vswhere) -and (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
if ($hasVc) { Write-Host "Visual Studio C++ build tools: installed." }
else {
  $missing += 'Visual Studio C++ build tools'
  if ($Check) { Write-Host "Visual Studio C++ build tools: missing." }
  else {
    Write-Host "Visual Studio C++ build tools: installing (this takes a while)..."
    winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-source-agreements --accept-package-agreements `
      --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  }
}

Need 'Rust (rustup)' 'rustup' 'Rustlang.Rustup'
Need 'ffmpeg' 'ffmpeg' 'Gyan.FFmpeg'
Need 'CMake' 'cmake' 'Kitware.CMake'
Need 'LLVM (libclang for whisper.cpp)' 'clang' 'LLVM.LLVM'
Need 'just' 'just' 'Casey.Just'
Need 'Node.js (JS tests only)' 'node' 'OpenJS.NodeJS.LTS'

if ((Have 'rustc')) {
  $v = (rustc --version).Split(' ')[1]
  if ([version]$v -lt [version]'1.88') {
    $missing += "Rust 1.88+ (have $v)"
    Write-Host "Rust $v is too old. Run: rustup update stable"
  }
}

Write-Host ""
if ($Check) {
  if ($missing.Count -gt 0) { Write-Host "Missing: $($missing -join ', ')"; exit 1 }
  Write-Host "Everything needed to build Booya is installed."; exit 0
}
Write-Host "Done. Open a new terminal so PATH changes apply, then:"
Write-Host "  just          list the tasks"
Write-Host "  just test     run the tests"
Write-Host "  just app      run the desktop app"
Write-Host "WebView2 is already part of Windows 11. On Windows 10, the app installer adds it if needed."
