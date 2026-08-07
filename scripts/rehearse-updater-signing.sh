#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-}"; KEEP="${KEEP_REHEARSAL:-0}"; OWN_OUT=0
if [[ -z "$OUT" ]]; then OUT="$(mktemp -d)"; OWN_OUT=1; fi
mkdir -p "$OUT/signing" "$OUT/payloads"
cleanup(){
  local rc="${1:-0}"
  if [[ "$KEEP" != 1 && "$OWN_OUT" == 1 ]]; then rm -rf "$OUT"; fi
  return "$rc"
}
trap 'cleanup $?' EXIT
key="$OUT/signing/rehearsal-key"
payloads=(linux-x86_64 darwin-x86_64 windows-x86_64)
# Tauri CLI versions have differed in whether they print the generated public-key
# path. Always use an isolated output directory and discover the resulting key;
# never echo the private key or include it in rehearsal evidence.
generate_log="$OUT/signing/generate.log"
if ! CI=true pnpm --dir "$ROOT_DIR/apps/desktop" tauri signer generate --ci -p rehearsal -w "$key" -f >"$generate_log" 2>&1; then
  echo "updater rehearsal: Tauri signer key generation failed" >&2
  grep -vE '^[A-Za-z0-9+/=]{40,}$' "$generate_log" | sed -E 's/(rehearsal-key|private key|secret)[^[:space:]]*/[redacted]/Ig' >&2 || true
  exit 1
fi
pub_file=""
for candidate in "$key.pub" "$OUT/signing/rehearsal-key.pub"; do
  if [[ -s "$candidate" ]]; then pub_file="$candidate"; break; fi
done
if [[ -z "$pub_file" ]]; then
  pub_file="$(find "$OUT/signing" -maxdepth 1 -type f -name '*.pub' -size +0c -print -quit)"
fi
if [[ -z "$pub_file" || ! -s "$pub_file" ]]; then
  echo "updater rehearsal: Tauri signer generated no public key" >&2
  echo "expected a .pub file beside the requested private-key path: $key" >&2
  echo "signer output (private material omitted):" >&2
  grep -vE '^[A-Za-z0-9+/=]{40,}$' "$generate_log" | sed -E 's/(rehearsal-key|private key|secret)[^[:space:]]*/[redacted]/Ig' >&2 || true
  exit 1
fi
pub="$(cat "$pub_file")"
[[ "$pub" != *PRIVATE* ]] || { echo 'updater rehearsal: public key output looked private' >&2; exit 1; }
rm -f "$generate_log"
for platform in "${payloads[@]}"; do
  file="$OUT/payloads/Infimount-${platform}.tar.gz"
  printf 'Infimount rehearsal %s\n' "$platform" > "$file"
  env -u TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PATH="$key" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=rehearsal \
    pnpm --dir "$ROOT_DIR/apps/desktop" tauri signer sign "$file" >/dev/null
  base64 -d "$file.sig" > "$file.sig.raw"
  cargo run -q -p infimount_core --bin verify_updater_signature -- "$pub" "$file" "$file.sig.raw" >/dev/null
  echo "verified $platform"
done
python3 - "$OUT" "$pub" <<'PY'
import json, pathlib, sys
out=pathlib.Path(sys.argv[1]); pub=sys.argv[2]
platforms={}
for p in ('linux-x86_64','darwin-x86_64','windows-x86_64'):
 f=out/'payloads'/f'Infimount-{p}.tar.gz'; platforms[p]={'url':f'http://127.0.0.1:0/v0.8.0-rc.1/{f.name}','signature':(f.with_name(f.name+'.sig')).read_text().strip()}
(out/'latest.json').write_text(json.dumps({'version':'0.8.0-rc.1','platforms':platforms},indent=2)+'\n')
(out/'signing/public.key').write_text(pub)
PY
# Negative tests: payload, signature, and public key tampering must fail.
for mode in payload signature key; do
 tmp="$OUT/negative-$mode"; mkdir -p "$tmp"; cp -a "$OUT/payloads" "$tmp/"; cp "$OUT/signing/public.key" "$tmp/public.key"
 case "$mode" in payload) printf tamper >> "$tmp/payloads/Infimount-linux-x86_64.tar.gz";; signature) printf tamper > "$tmp/payloads/Infimount-linux-x86_64.tar.gz.sig.raw";; key) printf tamper >> "$tmp/public.key";; esac
 if cargo run -q -p infimount_core --bin verify_updater_signature -- "$(cat "$tmp/public.key")" "$tmp/payloads/Infimount-linux-x86_64.tar.gz" "$tmp/payloads/Infimount-linux-x86_64.tar.gz.sig.raw" >/dev/null 2>&1; then echo "negative test failed: $mode" >&2; exit 1; fi
 echo "rejected tampered $mode"
done
