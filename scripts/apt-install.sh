#!/usr/bin/env bash
# Installs Ubuntu packages on a CI runner without hanging on a bad mirror.
#
# On 11 November 2026 the runners' Azure Ubuntu mirror stopped answering, and
# apt waited on every package in turn for close to an hour before the release
# build was cancelled. This script gives each try a time limit, retries each
# download, and if the first try fails it switches to the main Ubuntu mirror
# and tries once more.
#
# Usage: bash scripts/apt-install.sh pkg1 pkg2 ...
# Set APT_NO_RECOMMENDS=1 to skip recommended packages.
set -uo pipefail

APT_OPTS=(-o Acquire::Retries=3 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20 -o Dpkg::Use-Pty=0)
TRY_LIMIT="${APT_TRY_LIMIT:-600}"
EXTRA=()
[ "${APT_NO_RECOMMENDS:-0}" = "1" ] && EXTRA=(--no-install-recommends)

try_install() {
  sudo timeout "$TRY_LIMIT" apt-get "${APT_OPTS[@]}" update &&
    sudo DEBIAN_FRONTEND=noninteractive timeout "$TRY_LIMIT" apt-get "${APT_OPTS[@]}" install -y "${EXTRA[@]}" "$@"
}

if try_install "$@"; then
  exit 0
fi

echo "apt failed or ran out of time on the runner's mirror. Switching to archive.ubuntu.com and trying again."
for f in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
  [ -f "$f" ] && sudo sed -i 's#http://azure\.archive\.ubuntu\.com/ubuntu#http://archive.ubuntu.com/ubuntu#g' "$f"
done
try_install "$@"
