#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; OUT="${1:?asset directory}"; VERSION="${2:?version}"
rm -rf "$OUT"; mkdir -p "$OUT"
for name in Infimount-amd64.deb Infimount-x86_64.rpm Infimount-x86_64.AppImage Infimount.dmg Infimount.msi Infimount-setup.exe install.sh install.ps1; do printf 'Infimount rehearsal asset %s version %s\n' "$name" "$VERSION" > "$OUT/$name"; done
printf '#!/bin/sh\necho rehearsal installer\n' > "$OUT/install.sh"; chmod +x "$OUT/install.sh"
cp "$ROOT_DIR/scripts/install.ps1" "$OUT/install.ps1"
updater="$OUT/.updater"; bash "$ROOT_DIR/scripts/rehearse-updater-signing.sh" "$updater"
cp "$updater/payloads"/* "$OUT/"
rm -f "$OUT"/*.sig.raw
python3 - "$OUT/latest.json" "$OUT" "$VERSION" <<'PY'
import json,pathlib,sys
out=pathlib.Path(sys.argv[2]); version=sys.argv[3]; p={}
for platform in ('linux-x86_64','darwin-x86_64','windows-x86_64'):
 f=out/f'Infimount-{platform}.tar.gz'; p[platform]={'url':f'https://github.com/infimount/infimount/releases/download/v{version}/{f.name}','signature':(out/f'{f.name}.sig').read_text().strip()}
pathlib.Path(sys.argv[1]).write_text(json.dumps({'version':version,'platforms':p},indent=2)+'\n')
PY
bash "$ROOT_DIR/scripts/rehearse-sbom.sh" "$OUT" "$VERSION"
(cd "$OUT" && find . -maxdepth 1 -type f ! -name 'SHA256SUMS.txt' ! -name '*.sha256' -printf '%f\n' | sort | while read -r f; do sha256sum "$f"; done) > "$OUT/SHA256SUMS.txt"
while read -r sum f; do printf '%s  %s\n' "$sum" "$f" > "$OUT/$f.sha256"; done < "$OUT/SHA256SUMS.txt"
bash "$ROOT_DIR/scripts/check-release-assets.sh" "$OUT"
bash "$ROOT_DIR/scripts/check-updater-assets.sh" "$OUT" "$VERSION"
rm -rf "$updater"
