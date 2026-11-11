#!/usr/bin/env bash
# Installs Ubuntu packages on a CI runner without hanging on a bad mirror.
#
# On 11 November 2026 the runners' Azure Ubuntu mirror stopped answering, and
# apt waited on every package in turn for close to an hour before the release
# build was cancelled. archive.ubuntu.com did not answer from the runners
# either. This script gives each try a time limit, retries each download, and
# when a try fails it moves on to the next mirror in the list below.
#
# Usage: bash scripts/apt-install.sh pkg1 pkg2 ...
# Set APT_NO_RECOMMENDS=1 to skip recommended packages.
set -uo pipefail

APT_OPTS=(-o Acquire::Retries=3 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20 -o Dpkg::Use-Pty=0)
UPDATE_LIMIT="${APT_UPDATE_LIMIT:-180}"
INSTALL_LIMIT="${APT_INSTALL_LIMIT:-600}"
EXTRA=()
[ "${APT_NO_RECOMMENDS:-0}" = "1" ] && EXTRA=(--no-install-recommends)

# The runner's own mirror first, then mirrors run by other people, so one
# provider having a bad day does not stop the build.
MIRRORS=(
  ""
  "http://archive.ubuntu.com/ubuntu"
  "http://mirrors.edge.kernel.org/ubuntu"
  "http://mirror.math.princeton.edu/pub/ubuntu"
)

use_mirror() {
  local m="$1"
  echo "Trying the Ubuntu mirror at $m"
  # Newer runner images read their mirror from a mirror list file.
  if [ -f /etc/apt/apt-mirrors.txt ]; then
    printf '%s\tpriority:1\n' "$m" | sudo tee /etc/apt/apt-mirrors.txt >/dev/null
  fi
  for f in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
    [ -f "$f" ] || continue
    sudo sed -i -E "s#https?://(azure\.)?archive\.ubuntu\.com/ubuntu/?#$m/#g; s#https?://mirrors\.edge\.kernel\.org/ubuntu/?#$m/#g; s#https?://mirror\.math\.princeton\.edu/pub/ubuntu/?#$m/#g" "$f"
  done
}

for m in "${MIRRORS[@]}"; do
  [ -n "$m" ] && use_mirror "$m"
  if sudo timeout "$UPDATE_LIMIT" apt-get "${APT_OPTS[@]}" update &&
     sudo DEBIAN_FRONTEND=noninteractive timeout "$INSTALL_LIMIT" apt-get "${APT_OPTS[@]}" install -y "${EXTRA[@]}" "$@"; then
    exit 0
  fi
  echo "apt failed or ran out of time on this mirror."
done
echo "Every mirror failed."
exit 1
