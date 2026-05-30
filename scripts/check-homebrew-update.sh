#!/usr/bin/env bash
set -euo pipefail

VERSION_INPUT="${1:-${GITHUB_REF_NAME:-}}"
RELEASE_REPO="${RELEASE_REPO:-infimount/infimount}"
TAP_REPO="${HOMEBREW_TAP_REPO:-infimount/homebrew-infimount}"

fail() {
  echo "homebrew update check failed: $*" >&2
  exit 1
}

if [[ -z "$VERSION_INPUT" ]]; then
  fail "pass a release version or set GITHUB_REF_NAME"
fi
VERSION="${VERSION_INPUT#v}"
TAG="v${VERSION}"
TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t infimount-homebrew-check)"
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

git clone --depth 1 "https://github.com/${TAP_REPO}.git" "$TMP_DIR/tap" >/dev/null
(
  cd "$TMP_DIR/tap"
  RELEASE_REPO="$RELEASE_REPO" ./scripts/update-formula.sh "$VERSION"
  grep -Fq "version \"${VERSION}\"" Formula/infimount.rb || fail "Formula version was not updated to ${VERSION}"
  grep -Fq "version \"${VERSION}\"" Casks/infimount.rb || fail "Cask version was not updated to ${VERSION}"
  grep -Fq "releases/download/${TAG}/Infimount-x86_64.AppImage" Formula/infimount.rb || fail "Formula URL does not target ${TAG}"

  appimage_sha="$(curl -fsSL "https://github.com/${RELEASE_REPO}/releases/download/${TAG}/Infimount-x86_64.AppImage.sha256" | awk '{print $1}')"
  dmg_sha="$(curl -fsSL "https://github.com/${RELEASE_REPO}/releases/download/${TAG}/Infimount.dmg.sha256" | awk '{print $1}')"
  grep -Fq "sha256 \"${appimage_sha}\"" Formula/infimount.rb || fail "Formula sha256 does not match release AppImage checksum"
  grep -Fq "sha256 \"${dmg_sha}\"" Casks/infimount.rb || fail "Cask sha256 does not match release DMG checksum"
)

printf 'Homebrew update check passed for %s.\n' "$TAG"
