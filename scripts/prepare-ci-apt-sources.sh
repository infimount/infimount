#!/usr/bin/env bash
set -euo pipefail

sources_dir="${1:-/etc/apt/sources.list.d}"

if [[ ! -d "$sources_dir" ]]; then
  echo "APT source directory does not exist: $sources_dir" >&2
  exit 1
fi

shopt -s nullglob
for source in "$sources_dir"/*; do
  [[ -f "$source" ]] || continue

  # GitHub-hosted Ubuntu runners include third-party repositories that Infimount
  # does not use. A metadata/content mismatch in the Chrome repository blocked
  # the v0.8.1-rc.2 Linux release build twice before Infimount package
  # installation even began. Disable only that unrelated source so release
  # dependency installation depends on the Ubuntu repositories we actually use.
  if grep -Fq 'dl.google.com/linux/chrome-stable/deb' "$source"; then
    disabled="${source}.infimount-disabled"
    rm -f "$disabled"
    mv "$source" "$disabled"
    echo "Disabled unrelated Google Chrome APT source: $(basename "$source")"
  fi
done
shopt -u nullglob
