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

updated=0
for attempt in 1 2 3; do
  if sudo apt-get update; then
    updated=1
    break
  fi

  if [[ "$attempt" -eq 3 ]]; then
    break
  fi

  echo "APT index refresh failed (attempt $attempt/3); clearing partial lists and retrying." >&2
  sudo rm -rf /var/lib/apt/lists/partial/*
  sleep $((attempt * 3))
done

if [[ "$updated" -ne 1 ]]; then
  echo "APT index refresh failed after 3 attempts." >&2
  exit 1
fi

sudo apt-get install -y "$@"
