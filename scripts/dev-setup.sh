#!/usr/bin/env bash
# One command setup for working on Booya on Linux or macOS.
# Windows: use scripts/dev-setup.ps1 instead.
#
#   bash scripts/dev-setup.sh            install what's missing, asking first
#   bash scripts/dev-setup.sh --yes      install without asking
#   bash scripts/dev-setup.sh --check    only report what's missing (exit 1 if anything is)
#   bash scripts/dev-setup.sh --dry-run  print the install commands, run nothing
set -euo pipefail

mode=install
assume_yes=0
for a in "$@"; do
  case "$a" in
    --yes|-y) assume_yes=1 ;;
    --check) mode=check ;;
    --dry-run) mode=dry ;;
    -h|--help) sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown option: $a" >&2; exit 2 ;;
  esac
done

MIN_RUST="1.88"
root="$(cd "$(dirname "$0")/.." && pwd)"
missing=()

say() { printf '%s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

run() {
  if [ "$mode" = dry ]; then say "  would run: $*"; return 0; fi
  say "  running: $*"
  "$@"
}

confirm() {
  [ "$mode" = install ] || return 0
  [ "$assume_yes" = 1 ] && return 0
  printf '%s [y/N] ' "$1"
  read -r ans || ans=""
  case "$ans" in y|Y|yes) return 0 ;; *) return 1 ;; esac
}

ver_ge() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -1)" = "$2" ]; }

os="$(uname -s)"
pm=""
pkgs=()
case "$os" in
  Linux)
    if have apt-get; then
      pm=apt
      pkgs=(build-essential pkg-config curl ffmpeg cmake clang libclang-dev x11-utils
            libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libxdo-dev libssl-dev
            libx11-dev libxtst-dev libxi-dev libdbus-1-dev)
    elif have dnf; then
      pm=dnf
      pkgs=(gcc gcc-c++ make pkgconf-pkg-config curl ffmpeg cmake clang clang-devel xorg-x11-utils
            webkit2gtk4.1-devel gtk3-devel librsvg2-devel libxdo-devel openssl-devel
            libX11-devel libXtst-devel libXi-devel dbus-devel)
    elif have pacman; then
      pm=pacman
      pkgs=(base-devel pkgconf curl ffmpeg cmake clang xorg-xdpyinfo
            webkit2gtk-4.1 gtk3 librsvg xdotool openssl libx11 libxtst libxi dbus)
    else
      say "No apt, dnf or pacman found. Install the packages listed under Building in README.md by hand."
    fi
    ;;
  Darwin)
    pm=brew
    pkgs=(ffmpeg cmake node just)
    ;;
  *)
    say "This script is for Linux and macOS. On Windows run: powershell -ExecutionPolicy Bypass -File scripts\\dev-setup.ps1"
    exit 2
    ;;
esac

say "Booya dev setup ($os${pm:+, $pm})"
say ""

# 1. System packages
pkg_missing=()
case "$pm" in
  apt) for p in "${pkgs[@]}"; do dpkg -s "$p" >/dev/null 2>&1 || pkg_missing+=("$p"); done ;;
  dnf) for p in "${pkgs[@]}"; do rpm -q "$p" >/dev/null 2>&1 || pkg_missing+=("$p"); done ;;
  pacman) for p in "${pkgs[@]}"; do pacman -Qi "$p" >/dev/null 2>&1 || pkg_missing+=("$p"); done ;;
  brew)
    if ! xcode-select -p >/dev/null 2>&1; then missing+=("Xcode command line tools"); fi
    if ! have brew; then
      missing+=("Homebrew")
      say "Homebrew is missing. Install it from https://brew.sh, then run this again."
    else
      for p in "${pkgs[@]}"; do brew list --formula "$p" >/dev/null 2>&1 || pkg_missing+=("$p"); done
    fi
    ;;
esac

if [ "${#pkg_missing[@]}" -gt 0 ]; then
  missing+=("${pkg_missing[@]}")
  say "System packages missing: ${pkg_missing[*]}"
  if [ "$mode" != check ] && confirm "Install them now?"; then
    case "$pm" in
      apt) run sudo apt-get update; run sudo apt-get install -y --no-install-recommends "${pkg_missing[@]}" ;;
      dnf) run sudo dnf install -y "${pkg_missing[@]}" ;;
      pacman) run sudo pacman -S --needed --noconfirm "${pkg_missing[@]}" ;;
      brew) run brew install "${pkg_missing[@]}" ;;
    esac
  fi
else
  [ -n "$pm" ] && say "System packages: all there."
fi
if [ "$pm" = brew ] && ! xcode-select -p >/dev/null 2>&1 && [ "$mode" != check ]; then
  confirm "Install the Xcode command line tools?" && run xcode-select --install || true
fi

# 2. Rust
if have rustc; then
  rv="$(rustc --version | awk '{print $2}')"
  if ver_ge "$rv" "$MIN_RUST"; then
    say "Rust $rv: fine (need $MIN_RUST or newer)."
  else
    missing+=("Rust $MIN_RUST+ (have $rv)")
    say "Rust $rv is too old. Booya needs $MIN_RUST or newer."
    if [ "$mode" != check ] && have rustup && confirm "Run rustup update stable?"; then run rustup update stable; fi
  fi
else
  missing+=("Rust")
  say "Rust is missing."
  if [ "$mode" != check ] && confirm "Install it with rustup (https://rustup.rs)?"; then
    if [ "$mode" = dry ]; then say "  would run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
    else curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y; fi
  fi
fi

# 3. just, the task runner (optional, everything in the justfile also works by hand)
if have just; then
  say "just: installed."
elif [ "$pm" != brew ]; then
  missing+=("just")
  say "just is missing. It runs the tasks in the justfile (just test, just app, ...)."
  if [ "$mode" != check ] && confirm "Install just?"; then
    if [ "$pm" = pacman ]; then run sudo pacman -S --needed --noconfirm just
    elif [ "$pm" = apt ] && apt-cache show just >/dev/null 2>&1; then run sudo apt-get install -y just
    elif [ "$pm" = dnf ]; then run sudo dnf install -y just
    elif have cargo; then run cargo install just --locked
    else say "  Install Rust first, then: cargo install just --locked"; fi
  fi
fi

# 4. Node, only for the editor's JS tests
if have node; then
  say "Node $(node --version): fine for the JS tests."
else
  missing+=("node (only for the JS tests)")
  say "Node is missing. It's only needed for the editor's JS tests (just test-js)."
fi

say ""
if [ "$mode" = check ]; then
  if [ "${#missing[@]}" -gt 0 ]; then
    say "Missing: ${missing[*]}"
    say "Run: bash scripts/dev-setup.sh"
    exit 1
  fi
  say "Everything needed to build Booya is installed."
  exit 0
fi

say "Next:"
say "  cd $root"
if have just; then
  say "  just            list the tasks"
  say "  just test       Rust, JS and Python tests"
  say "  just app        run the desktop app"
else
  say "  cargo test -p booya-core -p booya-ai"
  say "  cargo run -p booya-app"
fi
