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
  [[ "$source" == *.infimount-disabled ]] && continue

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

  # The Azure-hosted Ubuntu APT mirror can stall for the full install timeout
  # while fetching the large WebKit2GTK package. Use Canonical's standard
  # signed Ubuntu archive instead. Do not touch third-party or security URLs.
  if grep -Fq 'azure.archive.ubuntu.com/ubuntu' "$source"; then
    sed -i \
      -e 's#http://azure\.archive\.ubuntu\.com/ubuntu#https://archive.ubuntu.com/ubuntu#g' \
      -e 's#https://azure\.archive\.ubuntu\.com/ubuntu#https://archive.ubuntu.com/ubuntu#g' \
      "$source"
    echo "Repointed Ubuntu Azure archive to official mirror: $(basename "$source")"
  fi
done
shopt -u nullglob
