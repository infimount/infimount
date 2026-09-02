#!/usr/bin/env bash
set -euo pipefail

# Rehearse the exact release signing policy without contacting GitHub or using secrets.
# Inputs: GITHUB_REF_NAME and the same *_KEY variables consumed by release.yml.
if [[ "${1:-}" == "--matrix" ]]; then
  names=(APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID WINDOWS_CERTIFICATE_BASE64 WINDOWS_CERTIFICATE_PASSWORD)
  base=(TAURI_SIGNING_PRIVATE_KEY=sentinel TAURI_SIGNING_PRIVATE_KEY_PASSWORD=sentinel)
  env GITHUB_REF_NAME=v0.8.0-rc.1 "${base[@]}" "$0" >/dev/null
  env GITHUB_REF_NAME=v0.8.0 "${base[@]}" "$0" >/dev/null
  env GITHUB_REF_NAME=v0.8.0 "${base[@]}" APPLE_CERTIFICATE=x APPLE_CERTIFICATE_PASSWORD=x APPLE_SIGNING_IDENTITY=x APPLE_ID=x APPLE_PASSWORD=x APPLE_TEAM_ID=x WINDOWS_CERTIFICATE_BASE64=x WINDOWS_CERTIFICATE_PASSWORD=x "$0" >/dev/null
  for name in "${names[@]}"; do
    args=(); for candidate in "${names[@]}"; do [[ "$candidate" == "$name" ]] || args+=("$candidate=x"); done
    if env GITHUB_REF_NAME=v0.8.0 "${base[@]}" "${args[@]}" "$0" >/dev/null 2>&1; then echo "matrix unexpectedly accepted partial platform signing" >&2; exit 1; fi
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
platform_names=(APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID WINDOWS_CERTIFICATE_BASE64 WINDOWS_CERTIFICATE_PASSWORD)
missing=()
for name in "${platform_names[@]}"; do
  [[ -n "${!name:-}" ]] || missing+=("$name")
done
if ((${#missing[@]} > 0 && ${#missing[@]} < ${#platform_names[@]})); then
  printf 'Platform signing must be fully configured or fully absent; missing: %s\n' "${missing[*]}" >&2
  exit 1
fi
platform_signed=false
if ((${#missing[@]} == 0)); then platform_signed=true; fi
if [[ "$ref" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo 'stable=true'
  echo "platform_signed=$platform_signed"
  if [[ "$platform_signed" == false ]]; then
    echo 'Stable release will be explicitly marked platform-unsigned; updater artifacts remain signed.'
  fi
else
  echo 'stable=false'
  echo "platform_signed=$platform_signed"
  echo 'Prerelease platform output may be unsigned; updater artifacts remain signed.'
fi
