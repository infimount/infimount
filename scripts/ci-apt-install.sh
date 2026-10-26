#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -eq 0 ]]; then
  echo "usage: $0 <apt-package> [apt-package ...]" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# GitHub-hosted Ubuntu images contain unrelated third-party APT sources.
# Keep those sources from making an Infimount release depend on their health.
sudo bash "$ROOT/scripts/prepare-ci-apt-sources.sh" /etc/apt/sources.list.d

# GitHub-hosted runners can hang in apt rather than returning a failure.
# Time-bound each attempt and kill unresponsive subprocesses, including apt's
# root-owned descendants. Timeout 124 is treated as an ordinary failed attempt.
APT_UPDATE_TIMEOUT_SECONDS="${APT_UPDATE_TIMEOUT_SECONDS:-180}"
APT_INSTALL_TIMEOUT_SECONDS="${APT_INSTALL_TIMEOUT_SECONDS:-360}"

updated=0
for attempt in 1 2 3; do
  if sudo timeout --signal=TERM --kill-after=15s "${APT_UPDATE_TIMEOUT_SECONDS}s" apt-get update; then
    updated=1
    break
  fi

  echo "APT index refresh failed or timed out (attempt $attempt/3)." >&2
  if [[ "$attempt" -eq 3 ]]; then
    break
  fi

  sudo rm -rf /var/lib/apt/lists/partial/*
  sleep $((attempt * 3))
done

if [[ "$updated" -ne 1 ]]; then
  echo "APT index refresh failed after 3 bounded attempts." >&2
  exit 1
fi

if ! sudo timeout --signal=TERM --kill-after=15s "${APT_INSTALL_TIMEOUT_SECONDS}s" apt-get install -y "$@"; then
  echo "APT package installation failed or timed out." >&2
  exit 1
fi
