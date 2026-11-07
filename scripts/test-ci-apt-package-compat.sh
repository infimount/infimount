#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/ci-apt-package-compat.sh
source "$ROOT/scripts/ci-apt-package-compat.sh"

# No network or real APT access. Simulate Ubuntu 24.04, Ubuntu 26.04,
# a transitional mirror containing both aliases, and missing drivers.
apt-cache() {
  [[ "${1:-}" == "policy" ]] || return 2
  local candidate="(none)"
  case "${CI_APT_TEST_CASE:-missing}:${2:-}" in
    old:webkit2gtk-driver|both:webkit2gtk-driver|both:webkitgtk-webdriver|new:webkitgtk-webdriver)
      candidate="1.0.0" ;;
  esac
  echo "${2:-}:"
  echo "  Installed: (none)"
  echo "  Candidate: $candidate"
  # A long policy listing must not be mistaken for a failed lookup because
  # its consumer closes the pipe before apt-cache finishes writing.
  for ((index = 0; index < 1024; index++)); do
    echo "    1.0-ubuntu.$index 500"
  done
}

CI_APT_TEST_CASE=old
ci_apt_resolve_packages libgtk-3-dev webkit2gtk-driver patchelf
[[ "${CI_APT_PACKAGES[*]}" == "libgtk-3-dev webkit2gtk-driver patchelf" ]]

CI_APT_TEST_CASE=new
ci_apt_resolve_packages libgtk-3-dev webkit2gtk-driver patchelf
[[ "${CI_APT_PACKAGES[*]}" == "libgtk-3-dev webkitgtk-webdriver patchelf" ]]

CI_APT_TEST_CASE=both
ci_apt_resolve_packages webkit2gtk-driver
[[ "${CI_APT_PACKAGES[*]}" == "webkit2gtk-driver" ]]

CI_APT_TEST_CASE=missing
if ci_apt_resolve_packages webkit2gtk-driver >/dev/null 2>&1; then
  echo "Uninstallable WebKit WebDriver was accepted." >&2
  exit 1
fi

CI_APT_TEST_CASE=missing
ci_apt_resolve_packages libgtk-3-dev patchelf
[[ "${CI_APT_PACKAGES[*]}" == "libgtk-3-dev patchelf" ]]

echo "WebKit WebDriver CI compatibility fixtures passed"
