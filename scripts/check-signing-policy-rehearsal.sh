#!/usr/bin/env bash
set -euo pipefail

# Rehearse the exact release signing policy without contacting GitHub or using secrets.
# Inputs: GITHUB_REF_NAME and the same *_KEY variables consumed by release.yml.
if [[ "${1:-}" == "--matrix" ]]; then
  apple=(APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID)
  windows=(WINDOWS_CERTIFICATE_BASE64 WINDOWS_CERTIFICATE_PASSWORD)
  base=(TAURI_SIGNING_PRIVATE_KEY=sentinel TAURI_SIGNING_PRIVATE_KEY_PASSWORD=sentinel)
  env GITHUB_REF_NAME=v0.8.0-rc.1 "${base[@]}" "$0" >/dev/null
  env GITHUB_REF_NAME=v0.8.0 "${base[@]}" "$0" >/dev/null
  env GITHUB_REF_NAME=v0.8.0 "${base[@]}" APPLE_CERTIFICATE=x APPLE_CERTIFICATE_PASSWORD=x APPLE_SIGNING_IDENTITY=x APPLE_ID=x APPLE_PASSWORD=x APPLE_TEAM_ID=x "$0" >/dev/null
  env GITHUB_REF_NAME=v0.8.0 "${base[@]}" WINDOWS_CERTIFICATE_BASE64=x WINDOWS_CERTIFICATE_PASSWORD=x "$0" >/dev/null
  env GITHUB_REF_NAME=v0.8.0 "${base[@]}" APPLE_CERTIFICATE=x APPLE_CERTIFICATE_PASSWORD=x APPLE_SIGNING_IDENTITY=x APPLE_ID=x APPLE_PASSWORD=x APPLE_TEAM_ID=x WINDOWS_CERTIFICATE_BASE64=x WINDOWS_CERTIFICATE_PASSWORD=x "$0" >/dev/null
  for nameset in apple windows; do
    if [[ "$nameset" == apple ]]; then names=("${apple[@]}"); else names=("${windows[@]}"); fi
    for name in "${names[@]}"; do
      args=(); for candidate in "${names[@]}"; do [[ "$candidate" == "$name" ]] || args+=("$candidate=x"); done
      if env GITHUB_REF_NAME=v0.8.0 "${base[@]}" "${args[@]}" "$0" >/dev/null 2>&1; then echo "matrix unexpectedly accepted partial ${nameset} signing" >&2; exit 1; fi
    done
  done
  if env -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD GITHUB_REF_NAME=v0.8.0-rc.1 "$0" >/dev/null 2>&1; then echo 'matrix unexpectedly accepted missing updater key' >&2; exit 1; fi
  for bad in v0.8.0+build v0 v0.8; do if env GITHUB_REF_NAME="$bad" "${base[@]}" "$0" >/dev/null 2>&1; then echo "matrix unexpectedly accepted $bad" >&2; exit 1; fi; done
  echo 'Signing-policy rehearsal matrix passed.'; exit 0
fi
ref="${GITHUB_REF_NAME:-}"
if [[ ! "$ref" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]]; then
  echo "Release tags must be SemVer without +build metadata: $ref" >&2; exit 1
fi
required=(TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD)
for name in "${required[@]}"; do
  [[ -n "${!name:-}" ]] || { echo "Every release requires updater signing; missing: $name" >&2; exit 1; }
done
apple_missing=()
for name in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
  [[ -n "${!name:-}" ]] || apple_missing+=("$name")
done
if ((${#apple_missing[@]} > 0 && ${#apple_missing[@]} < 6)); then
  printf 'Apple signing must be fully configured or fully absent; missing: %s\n' "${apple_missing[*]}" >&2
  exit 1
fi
windows_missing=()
for name in WINDOWS_CERTIFICATE_BASE64 WINDOWS_CERTIFICATE_PASSWORD; do
  [[ -n "${!name:-}" ]] || windows_missing+=("$name")
done
if ((${#windows_missing[@]} > 0 && ${#windows_missing[@]} < 2)); then
  printf 'Windows signing must be fully configured or fully absent; missing: %s\n' "${windows_missing[*]}" >&2
  exit 1
fi
apple_signed=false; windows_signed=false
if ((${#apple_missing[@]} == 0)); then apple_signed=true; fi
if ((${#windows_missing[@]} == 0)); then windows_signed=true; fi
if [[ "$ref" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo 'stable=true'
  echo "apple_signed=$apple_signed"
  echo "windows_signed=$windows_signed"
  if [[ "$apple_signed" == false || "$windows_signed" == false ]]; then
    echo 'Stable release will identify each platform signing status; updater artifacts remain signed.'
  fi
else
  echo 'stable=false'
  echo "apple_signed=$apple_signed"
  echo "windows_signed=$windows_signed"
  echo 'Prerelease platform output may be unsigned; updater artifacts remain signed.'
fi
